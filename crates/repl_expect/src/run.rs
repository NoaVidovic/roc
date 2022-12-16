use std::sync::{
    atomic::{AtomicBool, AtomicU32},
    Arc,
};

use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;
use inkwell::context::Context;
use roc_build::link::llvm_module_to_dylib;
use roc_can::expr::ExpectLookup;
use roc_collections::{MutSet, VecMap};
use roc_error_macros::internal_error;
use roc_gen_llvm::{
    llvm::{build::LlvmBackendMode, externs::add_default_roc_externs},
    run_roc::RocCallResult,
    run_roc_dylib,
};
use roc_intern::{GlobalInterner, SingleThreadedInterner};
use roc_load::{Expectations, MonomorphizedModule};
use roc_module::symbol::{Interns, ModuleId, Symbol};
use roc_mono::{ir::OptLevel, layout::Layout};
use roc_region::all::Region;
use roc_reporting::{error::expect::Renderer, report::RenderTarget};
use roc_target::TargetInfo;
use roc_types::subs::Subs;
use target_lexicon::Triple;

pub struct ExpectMemory<'a> {
    ptr: *mut u8,
    length: usize,
    shm_name: Option<std::ffi::CString>,
    _marker: std::marker::PhantomData<&'a ()>,
}

#[cfg(unix)]
unsafe fn allocate_shared_memory(
    file_name: &std::ffi::CStr,
    shm_size: usize,
    shm_flags: std::ffi::c_int,
) -> *mut libc::c_void {
    let shared_fd = libc::shm_open(file_name.as_ptr().cast(), shm_flags, 0o666);
    if shared_fd == -1 {
        internal_error!("failed to shm_open fd");
    }

    let mut stat: libc::stat = std::mem::zeroed();
    if libc::fstat(shared_fd, &mut stat) == -1 {
        internal_error!("failed to stat shared file, does it exist?");
    }
    if stat.st_size < shm_size as _ && libc::ftruncate(shared_fd, shm_size as _) == -1 {
        internal_error!("failed to truncate shared file, are the permissions wrong?");
    }

    let ptr = libc::mmap(
        std::ptr::null_mut(),
        shm_size,
        libc::PROT_WRITE | libc::PROT_READ,
        libc::MAP_SHARED,
        shared_fd,
        0,
    );

    if ptr as usize == usize::MAX {
        // ptr = -1
        roc_error_macros::internal_error!("failed to mmap shared pointer")
    }

    // fill the buffer with a fill pattern
    libc::memset(ptr, 0xAA, shm_size);

    ptr
}

#[cfg(windows)]
unsafe fn allocate_shared_memory(
    file_name: &std::ffi::CStr,
    shm_size: usize,
    shm_flags: std::ffi::c_int,
) -> *mut libc::c_void {
    use std::ffi::{c_char, c_int, c_ulong, c_void};

    type HANDLE = std::os::windows::raw::HANDLE;
    type LPCSTR = *const std::ffi::c_char;
    type DWORD = c_ulong;

    #[repr(C)]
    struct SECURITY_ATTRIBUTES {
        pub nLength: DWORD,
        pub lpSecurityDescriptor: *mut c_void,
        pub bInheritHandle: c_int,
    }

    const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

    // notes:
    //
    // docs at https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createfilemappinga
    //
    // - hFile: use https://doc.rust-lang.org/std/os/windows/io/trait.IntoRawHandle.html#tymethod.into_raw_handle
    // - lpFileMappingAttributes: configures inheritance of this memory. Not sure if relevant
    //      https://learn.microsoft.com/en-us/windows/win32/memory/file-mapping-security-and-access-rights
    extern "system" {
        fn CreateFileMappingA(
            hFile: HANDLE,
            lpFileMappingAttributes: *mut SECURITY_ATTRIBUTES,
            flProtect: DWORD,
            dwMaximumSizeHigh: DWORD,
            dwMaximumSizeLow: DWORD,
            lpName: LPCSTR,
        ) -> HANDLE;
    }

    // the flProtect value
    const PAGE_READWRITE: DWORD = 0x04;

    // low and high bits of the maximum size
    let dwMaximumSizeHigh = ((shm_size as u64) >> 32) as DWORD;
    let dwMaximumSizeLow = ((shm_size as u64) & 0xFFFFFFFF) as DWORD;

    // a name so we can find this mapping on the other side
    let lpName = file_name.as_ptr().cast();

    CreateFileMappingA(
        INVALID_HANDLE_VALUE,
        std::ptr::null_mut(),
        PAGE_READWRITE,
        dwMaximumSizeHigh,
        dwMaximumSizeLow,
        lpName,
    )
}

impl<'a> ExpectMemory<'a> {
    const SHM_SIZE: usize = 1024;

    #[cfg(test)]
    pub(crate) fn from_slice(slice: &mut [u8]) -> Self {
        Self {
            ptr: slice.as_mut_ptr(),
            length: slice.len(),
            shm_name: None,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn create_or_reuse_mmap(shm_name: &str) -> Self {
        let cstring = std::ffi::CString::new(shm_name).unwrap();
        Self::mmap_help(cstring, libc::O_RDWR | libc::O_CREAT)
    }

    // this will be used by expect-fx
    #[allow(unused)]
    fn reuse_mmap(&mut self) -> Option<Self> {
        let shm_name = self.shm_name.as_ref()?.clone();
        Some(Self::mmap_help(shm_name, libc::O_RDWR))
    }

    fn mmap_help(cstring: std::ffi::CString, shm_flags: i32) -> Self {
        let ptr = unsafe { allocate_shared_memory(&cstring, Self::SHM_SIZE, shm_flags) };

        // puts in the initial header
        let _ = ExpectSequence::new(ptr as *mut u8);

        Self {
            ptr: ptr.cast(),
            length: Self::SHM_SIZE,
            shm_name: Some(cstring),
            _marker: std::marker::PhantomData,
        }
    }

    fn set_shared_buffer(&mut self, lib: &libloading::Library) {
        let set_shared_buffer = run_roc_dylib!(lib, "set_shared_buffer", (*mut u8, usize), ());
        let mut result = RocCallResult::default();
        unsafe { set_shared_buffer((self.ptr, self.length), &mut result) };
    }

    pub fn wait_for_child(&self, sigchld: Arc<AtomicBool>) -> ChildProcessMsg {
        let sequence = ExpectSequence { ptr: self.ptr };
        sequence.wait_for_child(sigchld)
    }

    pub fn reset(&mut self) {
        let mut sequence = ExpectSequence { ptr: self.ptr };
        sequence.reset();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_inline_expects<'a, W: std::io::Write>(
    writer: &mut W,
    render_target: RenderTarget,
    arena: &'a Bump,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    lib: &libloading::Library,
    expectations: &mut VecMap<ModuleId, Expectations>,
    expects: ExpectFunctions<'_>,
) -> std::io::Result<(usize, usize)> {
    let shm_name = format!("/roc_expect_buffer_{}", std::process::id());
    let mut memory = ExpectMemory::create_or_reuse_mmap(&shm_name);

    run_expects_with_memory(
        writer,
        render_target,
        arena,
        interns,
        layout_interner,
        lib,
        expectations,
        expects,
        &mut memory,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_toplevel_expects<'a, W: std::io::Write>(
    writer: &mut W,
    render_target: RenderTarget,
    arena: &'a Bump,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    lib: &libloading::Library,
    expectations: &mut VecMap<ModuleId, Expectations>,
    expects: ExpectFunctions<'_>,
) -> std::io::Result<(usize, usize)> {
    let shm_name = format!("/roc_expect_buffer_{}", std::process::id());
    let mut memory = ExpectMemory::create_or_reuse_mmap(&shm_name);

    run_expects_with_memory(
        writer,
        render_target,
        arena,
        interns,
        layout_interner,
        lib,
        expectations,
        expects,
        &mut memory,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_expects_with_memory<'a, W: std::io::Write>(
    writer: &mut W,
    render_target: RenderTarget,
    arena: &'a Bump,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    lib: &libloading::Library,
    expectations: &mut VecMap<ModuleId, Expectations>,
    expects: ExpectFunctions<'_>,
    memory: &mut ExpectMemory,
) -> std::io::Result<(usize, usize)> {
    let mut failed = 0;
    let mut passed = 0;

    for expect in expects.fx {
        let result = run_expect_fx(
            writer,
            render_target,
            arena,
            interns,
            layout_interner,
            lib,
            expectations,
            memory,
            expect,
        )?;

        match result {
            true => passed += 1,
            false => failed += 1,
        }
    }

    memory.set_shared_buffer(lib);

    for expect in expects.pure {
        let result = run_expect_pure(
            writer,
            render_target,
            arena,
            interns,
            layout_interner,
            lib,
            expectations,
            memory,
            expect,
        )?;

        match result {
            true => passed += 1,
            false => failed += 1,
        }
    }

    Ok((failed, passed))
}

#[allow(clippy::too_many_arguments)]
fn run_expect_pure<'a, W: std::io::Write>(
    writer: &mut W,
    render_target: RenderTarget,
    arena: &'a Bump,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    lib: &libloading::Library,
    expectations: &mut VecMap<ModuleId, Expectations>,
    shared_memory: &mut ExpectMemory,
    expect: ToplevelExpect<'_>,
) -> std::io::Result<bool> {
    use roc_gen_llvm::try_run_jit_function;

    let sequence = ExpectSequence::new(shared_memory.ptr.cast());

    let result: Result<(), (String, _)> = try_run_jit_function!(lib, expect.name, (), |v: ()| v);

    let shared_memory_ptr: *const u8 = shared_memory.ptr.cast();

    if result.is_err() || sequence.count_failures() > 0 {
        let module_id = expect.symbol.module_id();
        let data = expectations.get_mut(&module_id).unwrap();

        let path = &data.path;
        let filename = data.path.to_owned();
        let source = std::fs::read_to_string(path).unwrap();

        let renderer = Renderer::new(arena, interns, render_target, module_id, filename, &source);

        if let Err((roc_panic_message, _roc_panic_tag)) = result {
            renderer.render_panic(writer, &roc_panic_message, expect.region)?;
        } else {
            let mut offset = ExpectSequence::START_OFFSET;

            for _ in 0..sequence.count_failures() {
                offset += render_expect_failure(
                    writer,
                    &renderer,
                    arena,
                    Some(expect),
                    expectations,
                    interns,
                    layout_interner,
                    shared_memory_ptr,
                    offset,
                )?;
            }
        }

        writeln!(writer)?;

        Ok(false)
    } else {
        Ok(true)
    }
}

#[allow(clippy::too_many_arguments)]
fn run_expect_fx<'a, W: std::io::Write>(
    _writer: &mut W,
    _render_target: RenderTarget,
    _arena: &'a Bump,
    _interns: &'a Interns,
    _layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    _lib: &libloading::Library,
    _expectations: &mut VecMap<ModuleId, Expectations>,
    _parent_memory: &mut ExpectMemory,
    _expect: ToplevelExpect<'_>,
) -> std::io::Result<bool> {
    todo!("expect fx is not yet implemented")
}

pub fn render_expects_in_memory<'a>(
    writer: &mut impl std::io::Write,
    arena: &'a Bump,
    expectations: &mut VecMap<ModuleId, Expectations>,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    memory: &ExpectMemory,
) -> std::io::Result<usize> {
    let shared_ptr = memory.ptr;

    let frame = ExpectFrame::at_offset(shared_ptr, ExpectSequence::START_OFFSET);
    let module_id = frame.module_id;

    let data = expectations.get_mut(&module_id).unwrap();
    let filename = data.path.to_owned();
    let source = std::fs::read_to_string(&data.path).unwrap();

    let renderer = Renderer::new(
        arena,
        interns,
        RenderTarget::ColorTerminal,
        module_id,
        filename,
        &source,
    );

    render_expect_failure(
        writer,
        &renderer,
        arena,
        None,
        expectations,
        interns,
        layout_interner,
        shared_ptr,
        ExpectSequence::START_OFFSET,
    )
}

pub fn render_dbgs_in_memory<'a>(
    writer: &mut impl std::io::Write,
    arena: &'a Bump,
    expectations: &mut VecMap<ModuleId, Expectations>,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    memory: &ExpectMemory,
) -> std::io::Result<usize> {
    let shared_ptr = memory.ptr;

    let frame = ExpectFrame::at_offset(shared_ptr, ExpectSequence::START_OFFSET);
    let module_id = frame.module_id;

    let data = expectations.get_mut(&module_id).unwrap();
    let filename = data.path.to_owned();
    let source = std::fs::read_to_string(&data.path).unwrap();

    let renderer = Renderer::new(
        arena,
        interns,
        RenderTarget::ColorTerminal,
        module_id,
        filename,
        &source,
    );

    render_dbg_failure(
        writer,
        &renderer,
        arena,
        expectations,
        interns,
        layout_interner,
        shared_ptr,
        ExpectSequence::START_OFFSET,
    )
}

fn split_expect_lookups(subs: &Subs, lookups: &[ExpectLookup]) -> Vec<Symbol> {
    lookups
        .iter()
        .filter_map(
            |ExpectLookup {
                 symbol,
                 var,
                 ability_info: _,
             }| {
                // mono will have dropped lookups that resolve to functions, so we should not keep
                // them either.
                if subs.is_function(*var) {
                    None
                } else {
                    Some(*symbol)
                }
            },
        )
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn render_dbg_failure<'a>(
    writer: &mut impl std::io::Write,
    renderer: &Renderer,
    arena: &'a Bump,
    expectations: &mut VecMap<ModuleId, Expectations>,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    start: *const u8,
    offset: usize,
) -> std::io::Result<usize> {
    // we always run programs as the host
    let target_info = (&target_lexicon::Triple::host()).into();

    let frame = ExpectFrame::at_offset(start, offset);
    let module_id = frame.module_id;

    let failure_region = frame.region;
    let dbg_symbol = unsafe { std::mem::transmute::<_, Symbol>(failure_region) };
    let expect_region = Some(Region::zero());

    let data = expectations.get_mut(&module_id).unwrap();

    let current = match data.dbgs.get(&dbg_symbol) {
        None => panic!("region {failure_region:?} not in list of dbgs"),
        Some(current) => current,
    };
    let failure_region = current.region;

    let subs = arena.alloc(&mut data.subs);

    let (offset, expressions, _variables) = crate::get_values(
        target_info,
        arena,
        subs,
        interns,
        layout_interner,
        start,
        frame.start_offset,
        1,
    );

    renderer.render_dbg(writer, &expressions, expect_region, failure_region)?;

    Ok(offset)
}

#[allow(clippy::too_many_arguments)]
fn render_expect_failure<'a>(
    writer: &mut impl std::io::Write,
    renderer: &Renderer,
    arena: &'a Bump,
    expect: Option<ToplevelExpect>,
    expectations: &mut VecMap<ModuleId, Expectations>,
    interns: &'a Interns,
    layout_interner: &Arc<GlobalInterner<'a, Layout<'a>>>,
    start: *const u8,
    offset: usize,
) -> std::io::Result<usize> {
    // we always run programs as the host
    let target_info = (&target_lexicon::Triple::host()).into();

    let frame = ExpectFrame::at_offset(start, offset);
    let module_id = frame.module_id;

    let failure_region = frame.region;
    let expect_region = expect.map(|e| e.region);

    let data = expectations.get_mut(&module_id).unwrap();

    let current = match data.expectations.get(&failure_region) {
        None => panic!("region {failure_region:?} not in list of expects"),
        Some(current) => current,
    };

    let symbols = split_expect_lookups(&data.subs, current);

    let (offset, expressions, variables) = crate::get_values(
        target_info,
        arena,
        &data.subs,
        interns,
        layout_interner,
        start,
        frame.start_offset,
        symbols.len(),
    );

    renderer.render_failure(
        writer,
        &mut data.subs,
        &symbols,
        &variables,
        &expressions,
        expect_region,
        failure_region,
    )?;

    Ok(offset)
}

struct ExpectSequence {
    ptr: *const u8,
}

impl ExpectSequence {
    const START_OFFSET: usize = 8 + 8 + 8;

    const COUNT_INDEX: usize = 0;
    const OFFSET_INDEX: usize = 1;
    const LOCK_INDEX: usize = 2;

    fn new(ptr: *mut u8) -> Self {
        unsafe {
            let ptr = ptr as *mut usize;
            std::ptr::write_unaligned(ptr.add(Self::COUNT_INDEX), 0);
            std::ptr::write_unaligned(ptr.add(Self::OFFSET_INDEX), Self::START_OFFSET);
            std::ptr::write_unaligned(ptr.add(Self::LOCK_INDEX), 0);
        }

        Self {
            ptr: ptr as *const u8,
        }
    }

    fn count_failures(&self) -> usize {
        unsafe { *(self.ptr as *const usize).add(Self::COUNT_INDEX) }
    }

    fn wait_for_child(&self, sigchld: Arc<AtomicBool>) -> ChildProcessMsg {
        use std::sync::atomic::Ordering;
        let ptr = self.ptr as *const u32;
        let atomic_ptr: *const AtomicU32 = unsafe { ptr.add(5).cast() };
        let atomic = unsafe { &*atomic_ptr };

        loop {
            if sigchld.load(Ordering::Relaxed) {
                break ChildProcessMsg::Terminate;
            }

            match atomic.load(Ordering::Acquire) {
                0 => std::hint::spin_loop(),
                1 => break ChildProcessMsg::Expect,
                2 => break ChildProcessMsg::Dbg,
                n => panic!("invalid atomic value set by the child: {:#x}", n),
            }
        }
    }

    fn reset(&mut self) {
        unsafe {
            let ptr = self.ptr as *mut usize;
            std::ptr::write_unaligned(ptr.add(Self::COUNT_INDEX), 0);
            std::ptr::write_unaligned(ptr.add(Self::OFFSET_INDEX), Self::START_OFFSET);
            std::ptr::write_unaligned(ptr.add(Self::LOCK_INDEX), 0);
        }
    }
}

pub enum ChildProcessMsg {
    Expect = 1,
    Dbg = 2,
    Terminate = 3,
}

struct ExpectFrame {
    region: Region,
    module_id: ModuleId,

    start_offset: usize,
}

impl ExpectFrame {
    fn at_offset(start: *const u8, offset: usize) -> Self {
        let region_bytes: [u8; 8] = unsafe { *(start.add(offset).cast()) };
        let region: Region = unsafe { std::mem::transmute(region_bytes) };

        let module_id_bytes: [u8; 4] = unsafe { *(start.add(offset + 8).cast()) };
        let module_id: ModuleId = unsafe { std::mem::transmute(module_id_bytes) };

        // skip to frame
        let start_offset = offset + 8 + 4;

        Self {
            region,
            module_id,
            start_offset,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ToplevelExpect<'a> {
    pub name: &'a str,
    pub symbol: Symbol,
    pub region: Region,
}

#[derive(Debug)]
pub struct ExpectFunctions<'a> {
    pub pure: BumpVec<'a, ToplevelExpect<'a>>,
    pub fx: BumpVec<'a, ToplevelExpect<'a>>,
}

pub fn expect_mono_module_to_dylib<'a>(
    arena: &'a Bump,
    target: Triple,
    loaded: MonomorphizedModule<'a>,
    opt_level: OptLevel,
    mode: LlvmBackendMode,
) -> Result<
    (
        libloading::Library,
        ExpectFunctions<'a>,
        SingleThreadedInterner<'a, Layout<'a>>,
    ),
    libloading::Error,
> {
    let target_info = TargetInfo::from(&target);

    let MonomorphizedModule {
        toplevel_expects,
        procedures,
        interns,
        layout_interner,
        ..
    } = loaded;

    let context = Context::create();
    let builder = context.create_builder();
    let module = arena.alloc(roc_gen_llvm::llvm::build::module_from_builtins(
        &target, &context, "",
    ));

    let module = arena.alloc(module);
    let (module_pass, _function_pass) =
        roc_gen_llvm::llvm::build::construct_optimization_passes(module, opt_level);

    let (dibuilder, compile_unit) = roc_gen_llvm::llvm::build::Env::new_debug_info(module);

    // Compile and add all the Procs before adding main
    let env = roc_gen_llvm::llvm::build::Env {
        arena,
        layout_interner: &layout_interner,
        builder: &builder,
        dibuilder: &dibuilder,
        compile_unit: &compile_unit,
        context: &context,
        interns,
        module,
        target_info,
        mode,
        // important! we don't want any procedures to get the C calling convention
        exposed_to_host: MutSet::default(),
    };

    // Add roc_alloc, roc_realloc, and roc_dealloc, since the repl has no
    // platform to provide them.
    add_default_roc_externs(&env);

    let capacity = toplevel_expects.pure.len() + toplevel_expects.fx.len();
    let mut expect_symbols = BumpVec::with_capacity_in(capacity, env.arena);

    expect_symbols.extend(toplevel_expects.pure.keys().copied());
    expect_symbols.extend(toplevel_expects.fx.keys().copied());

    let expect_names = roc_gen_llvm::llvm::build::build_procedures_expose_expects(
        &env,
        opt_level,
        &expect_symbols,
        procedures,
    );

    let expects_fx = bumpalo::collections::Vec::from_iter_in(
        toplevel_expects
            .fx
            .into_iter()
            .zip(expect_names.iter().skip(toplevel_expects.pure.len()))
            .map(|((symbol, region), name)| ToplevelExpect {
                symbol,
                region,
                name,
            }),
        env.arena,
    );

    let expects_pure = bumpalo::collections::Vec::from_iter_in(
        toplevel_expects
            .pure
            .into_iter()
            .zip(expect_names.iter())
            .map(|((symbol, region), name)| ToplevelExpect {
                symbol,
                region,
                name,
            }),
        env.arena,
    );

    let expects = ExpectFunctions {
        pure: expects_pure,
        fx: expects_fx,
    };

    env.dibuilder.finalize();

    // we don't use the debug info, and it causes weird errors.
    module.strip_debug_info();

    // Uncomment this to see the module's un-optimized LLVM instruction output:
    // env.module.print_to_stderr();

    module_pass.run_on(env.module);

    // Uncomment this to see the module's optimized LLVM instruction output:
    // env.module.print_to_stderr();

    // Verify the module
    if let Err(errors) = env.module.verify() {
        let path = std::env::temp_dir().join("test.ll");
        env.module.print_to_file(&path).unwrap();
        panic!(
            "Errors defining module:\n{}\n\nUncomment things nearby to see more details. IR written to `{:?}`",
            errors.to_string(), path,
        );
    }

    llvm_module_to_dylib(env.module, &target, opt_level).map(|lib| (lib, expects, layout_interner))
}
