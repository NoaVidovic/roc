interface List
    exposes [ List, map, fold ]
    imports []

## Types

## A sequential list of values.
##
## >>> [ 1, 2, 3 ] # a list of numbers
##
## >>> [ "a", "b", "c" ] # a list of strings
##
## >>> [ [ 1.1 ], [], [ 2.2, 3.3 ] ] # a list of lists of floats
##
## The list [ 1, "a" ] gives an error, because each element in a list must have
## the same type. If you want to put a mix of #Int and #Str values into a list, try this:
##
## ```
## mixedList : List [ IntElem Int, StrElem Str ]*
## mixedList = [ IntElem 1, IntElem 2, StrElem "a", StrElem "b" ]
## ```
##
## The maximum size of a #List is limited by the amount of heap memory available
## to the current process. If there is not enough memory available, attempting to
## create the list could crash. (On Linux, where [overcommit](https://www.etalabs.net/overcommit.html)
## is normally enabled, not having enough memory could result in the list appearing
## to be created just fine, but then crashing later.)
##
## > The theoretical maximum length for a list created in Roc is
## > #Int.highestLen divided by 2. Attempting to create a list bigger than that
## > in Roc code will always fail, although in practice it is likely to fail
## > at much smaller lengths due to insufficient memory being available.
##
## ## Performance Details
##
## Under the hood, a list is a record containing a `len : Len` field as well
## as a pointer to a flat list of bytes.
##
## This is not a [persistent data structure](https://en.wikipedia.org/wiki/Persistent_data_structure),
## so copying it is not cheap! The reason #List is designed this way is because:
##
## * Copying small lists is typically slightly faster than copying small persistent data structures. This is because, at small sizes, persistent data structures are usually thin wrappers around flat lists anyway. They don't start conferring copying advantages until crossing a certain minimum size threshold.
## Many list operations are no faster with persistent data structures. For example, even if it were a persistent data structure, #List.map, #List.fold, and #List.keepIf would all need to traverse every element in the list and build up the result from scratch.
## * Roc's compiler optimizes many list operations into in-place mutations behind the scenes, depending on how the list is being used. For example, #List.map, #List.keepIf, and #List.set can all be optimized to perform in-place mutations.
## * If possible, it is usually best for performance to use large lists in a way where the optimizer can turn them into in-place mutations. If this is not possible, a persistent data structure might be faster - but this is a rare enough scenario that it would not be good for the average Roc program's performance if this were the way #List worked by default. Instead, you can look outside Roc's standard modules for an implementation of a persistent data structure - likely built using #List under the hood!
List elem : @List elem

## Initialize

## A list with a single element in it.
##
## This is useful in pipelines, like so:
##
##     websites =
##         Str.concat domain ".com"
##             |> List.single
##
single : elem -> List elem

## An empty list.
empty : List *

## Returns a list with the given length, where every element is the given value.
##
##
repeat : elem, Len -> List elem

## Returns a list of all the integers between one and another,
## including both of the given numbers.
##
## >>> List.range 2 8
range : Int a, Int a -> List (Int a)

## Transform

## Returns the list with its elements reversed.
##
## >>> List.reverse [ 1, 2, 3 ]
reverse : List elem -> List elem

## Sorts a list using a function which specifies how two elements are ordered.
##
##
sort : List elem, (elem, elem -> [ Lt, Eq, Gt ]) -> List elem

## Convert each element in the list to something new, by calling a conversion
## function on each of them. Then return a new list of the converted values.
##
## > List.map [ 1, 2, 3 ] (\num -> num + 1)
##
## > List.map [ "", "a", "bc" ] Str.isEmpty
##
## `map` functions like this are common in Roc, and they all work similarly.
## See for example #Result.map, #Set.map, and #Map.map.
map : List before, (before -> after) -> List after

## This works the same way as #List.map, except it also passes the index
## of the element to the conversion function.
indexedMap : List before, (before, Int -> after) -> List after

## Add a single element to the end of a list.
##
## >>> List.append [ 1, 2, 3 ] 4
##
## >>> [ 0, 1, 2 ]
## >>>     |> List.append 3
append : List elem, elem -> List elem

## Add a single element to the beginning of a list.
##
## >>> List.prepend [ 1, 2, 3 ] 0
##
## >>> [ 2, 3, 4 ]
## >>>     |> List.prepend 1
prepend : List elem, elem -> List elem

## Put two lists together.
##
## >>> List.concat [ 1, 2, 3 ] [ 4, 5 ]
##
## >>> [ 0, 1, 2 ]
## >>>     |> List.concat [ 3, 4 ]
concat : List elem, List elem -> List elem

## Join the given lists together into one list.
##
## >>> List.join [ [ 1, 2, 3 ], [ 4, 5 ], [], [ 6, 7 ] ]
##
## >>> List.join [ [], [] ]
##
## >>> List.join []
join : List (List elem) -> List elem

joinMap : List before, (before -> List after) -> List after

## Like #List.join, but only keeps elements tagged with `Ok`. Elements
## tagged with `Err` are dropped.
##
## This can be useful after using an operation that returns a #Result
## on each element of a list, for example #List.first:
##
## >>> [ [ 1, 2, 3 ], [], [], [ 4, 5 ] ]
## >>>     |> List.map List.first
## >>>     |> List.joinOks
joinOks : List (Result elem *) -> List elem

## Iterates over the shortest of the given lists and returns a list of `Pair`
## tags, each wrapping one of the elements in that list, along with the elements
## in the same position in # the other lists.
##
## >>> List.zip [ "a1", "b1" "c1" ] [ "a2", "b2" ] [ "a3", "b3", "c3" ]
##
## Accepts up to 8 lists.
##
## > For a generalized version that returns whatever you like, instead of a `Pair`,
## > see `zipMap`.
zip :
    List a, List b, -> List [ Pair a b ]*
    List a, List b, List c, -> List [ Pair a b c ]*
    List a, List b, List c, List d  -> List [ Pair a b c d ]*

## Like `zip` but you can specify what to do with each element.
##
## More specifically, [repeat what zip's docs say here]
##
## >>> List.zipMap [ 1, 2, 3 ] [ 0, 5, 4 ] [ 2, 1 ] \num1 num2 num3 -> num1 + num2 - num3
##
## Accepts up to 8 lists.
zipMap :
    List a, List b, (a, b) -> List c |
    List a, List b, List c, (a, b, c) -> List d |
    List a, List b, List c, List d, (a, b, c, d) -> List e


## Filter

## Run the given function on each element of a list, and return all the
## elements for which the function returned `True`.
##
## >>> List.keepIf [ 1, 2, 3, 4 ] (\num -> num > 2)
##
## ## Performance Details
##
## #List.keepIf always returns a list that takes up exactly the same amount
## of memory as the original, even if its length decreases. This is becase it
## can't know in advance exactly how much space it will need, and if it guesses a
## length that's too low, it would have to re-allocate.
##
## (If you want to do an operation like this which reduces the memory footprint
## of the resulting list, you can do two passes over the lis with #List.fold - one
## to calculate the precise new size, and another to populate the new list.)
##
## If given a unique list, #List.keepIf will mutate it in place to assemble the appropriate list.
## If that happens, this function will not allocate any new memory on the heap.
## If all elements in the list end up being kept, Roc will return the original
## list unaltered.
##
keepIf : List elem, (elem -> [True, False]) -> List elem

## Run the given function on each element of a list, and return all the
## elements for which the function returned `False`.
##
## >>> List.dropIf [ 1, 2, 3, 4 ] (\num -> num > 2)
##
## ## Performance Details
##
## #List.dropIf has the same performance characteristics as #List.keepIf.
## See its documentation for details on those characteristics!
dropIf : List elem, (elem -> [True, False]) -> List elem

## Takes the requested number of elements from the front of a list
## and returns them.
##
## >>> take 5 [ 1, 2, 3, 4, 5, 6, 7, 8 ]
##
## If there are fewer elements in the list than the requeted number,
## returns the entire list.
##
## >>> take 5 [ 1, 2 ]
take : List elem, Int -> List elem

## Access

## Returns the first element in the list, or `ListWasEmpty` if the list was empty.
first : List elem -> Result elem [ ListWasEmpty ]*

## Returns the last element in the list, or `ListWasEmpty` if the list was empty.
last : List elem -> Result elem [ ListWasEmpty ]*

## This takes a #Len because the maximum length of a #List is a #Len value,
## so #Len lets you specify any position up to the maximum length of
## the list.
get : List elem, Len -> Result elem [ OutOfBounds ]*

max : List (Num a) -> Result (Num a) [ ListWasEmpty ]*

min : List (Num a) -> Result (Num a) [ ListWasEmpty ]*

## Modify

## This takes a #Len because the maximum length of a #List is a #Len value,
## so #Len lets you specify any position up to the maximum length of
## the list.
set : List elem, Len, elem -> List elem

## Add a new element to the end of a list.
##
## Returns a new list with the given element as its last element.
##
## ## Performance Details
##
## When given a Unique list, this adds the new element in-place if possible.
## This is only possible if the list has enough capacity. Otherwise, it will
## have to *clone and grow*. See the section on [capacity](#capacity) in this
## module's documentation.
append : List elem, elem -> List elem

## Add a new element to the beginning of a list.
##
## Returns a new list with the given element as its first element.
##
## ## Performance Details
##
## This always clones the entire list, even when given a Unique list. That means
## it runs about as fast as #List.addLast when both are given a Shared list.
##
## If you have a Unique list instead, #List.append will run much faster than
## #List.prepend except in the specific case where the list has no excess capacity,
## and needs to *clone and grow*. In that uncommon case, both #List.append and
## #List.prepend will run at about the same speed—since #List.prepend always
## has to clone and grow.
##
##         | Unique list                    | Shared list    |
##---------+--------------------------------+----------------+
## append  | in-place given enough capacity | clone and grow |
## prepend | clone and grow                 | clone and grow |
prepend : List elem, elem -> List elem

## Remove the last element from the list.
##
## Returns both the removed element as well as the new list (with the removed
## element missing), or `Err ListWasEmpty` if the list was empty.
##
## Here's one way you can use this:
##
##     when List.pop list is
##         Ok { others, last } -> ...
##         Err ListWasEmpty -> ...
##
## ## Performance Details
##
## Calling #List.pop on a Unique list runs extremely fast. It's essentially
## the same as a #List.last except it also returns the #List it was given,
## with its length decreased by 1.
##
## In contrast, calling #List.pop on a Shared list creates a new list, then
## copies over every element in the original list except the last one. This
## takes much longer.
dropLast : List elem -> Result { others : List elem, last : elem } [ ListWasEmpty ]*

##
## Here's one way you can use this:
##
##     when List.pop list is
##         Ok { others, last } -> ...
##         Err ListWasEmpty -> ...
##
## ## Performance Details
##
## When calling either #List.dropFirst or #List.dropLast on a Unique list, #List.dropLast
## runs *much* faster. This is because for #List.dropLast, removing the last element
## in-place is as easy as reducing the length of the list by 1. In contrast,
## removing the first element from the list involves copying every other element
## in the list into the position before it - which is massively more costly.
##
## In the case of a Shared list,
##
##           | Unique list                      | Shared list                     |
##-----------+----------------------------------+---------------------------------+
## dropFirst | #List.last + length change       | #List.last + clone rest of list |
## dropLast  | #List.last + clone rest of list  | #List.last + clone rest of list |
dropFirst : List elem -> Result { first: elem, others : List elem } [ ListWasEmpty ]*

## Drops the given number of elements from the end of the list.
##
## Returns a new list without the dropped elements.
##
## To remove elements from a list while also returning a list of the removed
## elements, use #List.split.
##
## To remove elements from the beginning of the list, use #List.dropFromFront.
##
## ## Performance Details
##
## When given a Unique list, this runs extremely fast. It subtracts the given
## number from the list's length (down to a minimum of 0) in-place, and that's it.
##
## In fact, `List.drop 1 list` runs faster than `List.dropLast list` when given
## a Unique list, because #List.dropLast returns the element it dropped -
## which introduces a conditional bounds check as well as a memory load.
drop : List elem, Len -> List elem

## Drops the given number of elements from the front of the list.
##
## Returns a new list without the dropped elements.
##
## To remove elements from a list while also returning a list of the removed
## elements, use #List.split.
##
## ## Performance Details
##
## When given a Unique list, this runs extremely fast. It subtracts the given
## number from the list's length (down to a minimum of 0) in-place, and that's it.
##
## In fact, `List.drop 1 list` runs faster than `List.dropLast list` when given
## a Unique list, because #List.dropLast returns the element it dropped -
## which introduces a conditional bounds check as well as a memory load.
dropFromFront : List elem, Len -> List elem

## Deconstruct

## Splits the list into two lists, around the given index.
##
## The returned lists are labeled `before` and `others`. The `before` list will
## contain all the elements whose index in the original list was **less than**
## than the given index, # and the `others` list will be all the others. (This
## means if you give an index of 0, the `before` list will be empty and the
## `others` list will have the same elements as the original list.)
split : List elem, Len -> { before: List elem, others: List elem }

## Build a value using each element in the list.
##
## Starting with a given `state` value, this walks through each element in the
## list from first to last, running a given `step` function on that element
## which updates the `state`. It returns the final `state` at the end.
##
## You can use it in a pipeline:
##
##     [ 2, 4, 8 ]
##         |> List.walk { start: 0, step: Num.add }
##
## This returns 14 because:
## * `state` starts at 0 (because of `start: 0`)
## * Each `step` runs `Num.add state elem`, and the return value becomes the new `state`.
##
## Here is a table of how `state` changes as #List.walk walks over the elements
## `[ 2, 4, 8 ]` using #Num.add as its `step` function to determine the next `state`.
##
## `state` | `elem` | `step state elem` (`Num.add state elem`)
## --------+--------+-----------------------------------------
## 0       |        |
## 0       | 2      | 2
## 2       | 4      | 6
## 6       | 8      | 14
##
## So `state` goes through these changes:
## 1. `0` (because of `start: 0`)
## 2. `1` (because of `Num.add state elem` with `state` = 0 and `elem` = 1
##
##     [ 1, 2, 3 ]
##         |> List.walk { start: 0, step: Num.sub }
##
## This returns -6 because
##
## Note that in other languages, `walk` is sometimes called `reduce`,
## `fold`, `foldLeft`, or `foldl`.
walk : List elem, { start : state, step : (state, elem -> state) } -> state

## Note that in other languages, `walkBackwards` is sometimes called `reduceRight`,
## `fold`, `foldRight`, or `foldr`.
walkBackwards : List elem, { start : state, step : (state, elem -> state ]) } -> state

## Same as #List.walk, except you can stop walking early.
##
## ## Performance Details
##
## Compared to #List.walk, this can potentially visit fewer elements (which can
## improve performance) at the cost of making each step take longer.
## However, the added cost to each step is extremely small, and can easily
## be outweighed if it results in skipping even a small number of elements.
##
## As such, it is typically better for performance to use this over #List.walk
## if returning `Done` earlier than the last element is expected to be common.
walkUntil : List elem, { start : state, step : (state, elem -> [ Continue state, Done state ]) } -> state

# Same as #List.walkBackwards, except you can stop walking early.
walkBackwardsUntil : List elem, { start : state, step : (state, elem -> [ Continue state, Done state ]) } -> state

## Check

## Returns the length of the list - the number of elements it contains.
##
## One #List can store up to 2,147,483,648 elements (just over 2 billion), which
## is exactly equal to the highest valid #I32 value. This means the #U32 this function
## returns can always be safely converted to an #I32 without losing any data.
len : List * -> Len

isEmpty : List * -> Bool

contains : List elem, elem -> Bool

all : List elem, (elem -> Bool) -> Bool

any : List elem, (elem -> Bool) -> Bool
