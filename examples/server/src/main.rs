use futures::{Future, FutureExt};
use hyper::{Body, Request, Response, Server, StatusCode};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use tokio::task::spawn_blocking;

const LISTEN_ON_PORT: u16 = 8000;

fn call_roc(_req_bytes: &[u8]) -> (StatusCode, Vec<u8>) {
    // TODO install signal handlers for SIGSEGV, SIGILL, SIGBUS, and SIGFPE, either here or perhaps at the top level
    (StatusCode::OK, Vec::new()) // TODO convert roc_bytes to RocList<u8>, call roc_mainForHost, and convert from its RocList<u8> response
}

async fn handle(req: Request<Body>) -> Response<Body> {
    match hyper::body::to_bytes(req.into_body()).await {
        Ok(req_body) => {
            spawn_blocking(move || {
                let (status_code, resp_bytes) = call_roc(&req_body);

                Response::builder()
                    .status(status_code) // TODO get status code from Roc too
                    .body(Body::from(resp_bytes))
                    .unwrap() // TODO don't unwrap() here
            })
            .then(|resp| async {
                resp.unwrap() // TODO don't unwrap here
            })
            .await
        }
        Err(_) => todo!(), // TODO
    }
}

/// Translate Rust panics in the given Future into 500 errors
async fn handle_panics(
    fut: impl Future<Output = Response<Body>>,
) -> Result<Response<Body>, Infallible> {
    match AssertUnwindSafe(fut).catch_unwind().await {
        Ok(response) => Ok(response),
        Err(_panic) => {
            let error = Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body("Panic detected!".into())
                .unwrap(); // TODO don't unwrap here

            Ok(error)
        }
    }
}

#[tokio::main]
async fn main() {
    let addr = SocketAddr::from(([127, 0, 0, 1], LISTEN_ON_PORT));
    let server = Server::bind(&addr).serve(hyper::service::make_service_fn(|_conn| async {
        Ok::<_, Infallible>(hyper::service::service_fn(|req| handle_panics(handle(req))))
    }));

    if let Err(e) = server.await {
        eprintln!("Error initializing Rust `hyper` server: {}", e); // TODO improve this
    }
}
