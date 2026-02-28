# Bitena

A small, extremely fast, lock-free thread-safe arena bump allocator that can hand out multiple
mutable elements, structs, slices, or read-only &strs from a single pre-allocated block.

## What is an Arena?

An arena allocator is a memory allocation strategy that pre-allocates a large block
of memory once, and can then hand out sub-allocations from that block sequentially.
Bitena is are much faster than normal memory allocations because:

  - **Bulk Allocation**: The entire arena is allocated all at once
  - **No Fragmentation**: Memory is allocated sequentially
  - **Fast Bookkeeping**: No complex tracking/reallocations
  - **Simplified Deallocation**: The entire arena is freed simultaneously

`Bitena` is special in that, because of its design, it is not subject
to the same use after free or overlapped memory bugs that are possible with
some other bump allocators.

## Quick Start

Add the following to your `Cargo.toml`:

```toml
[dependencies]
bitena = "0.1"
```

```rust
  use bitena::Bitena;
  fn main() {
    let mut bitena = Bitena::new(1024).expect("Failed to allocate memory");
    let num = bitena.alloc(42u32);
    let stnum = format!("Num: {}", *num);
    let s = bitena.alloc_str(&stnum);
    println!("{}  {:?}", *num, s);
  }
```

# The API

## new(byte_capacity)
Allocate a new Arena with a specified capacity.

## alloc(item) or try_alloc(item)
Allocate an element or structure in the Arena

## alloc_slice(init_value, items) or try_alloc_slice(init_value, items)
Allocate a slice vector of elements

## alloc_str(&str) or try_alloc_str(&str)
Store a &str in the Arena

## reset()
Reset the arena. This requires that all allocations are vacated, and
re-initializes the Arena to it's brand new state.

## Tradeoffs

  - Individual Items are not resizeable. Each element or item allocated from
    the arena is a fixed size. You need to individually Box<T> any items, 
    (Strings, Vecs, Fat Pointers, file handles, etc) to avoid leaking memory.

  - The entire arena will be dropped in a single operation. Individual Drop
    operations will not be performed on the Arena's contents. This then will
    leak any memory separately allocated as with Strings and Vecs.

  - **No item Reclamation**: Any unused allocations are stuck until
    the whole arena is dropped or reset().

  - **Fixed Size**: The arena has a set fixed size that doesn't grow.

# MIRI to the Rescue

Miri detected attempted memory leaks with String, and Vec in our tests.

```ignore
  cargo +nightly miri run
```


# Use Cases:

 - **Long-lived Data**: Perform one alloc from the system, and break that into
   all the allocations your need for the life of your program

 - **Short-lived Processing**: Temporary allocations for a process... encoding,
   parsing, compiling, translation, etc. All the memory can be reused with reset()
   or set or returned/deallocated at the end of processing.

 - **Saving Space**: Many system allocation schemes allocate in page sized blocks
   so freed memory can be more efficiently managed for reallocation. Arena fills
   every byte it can given alignment requirements.


# Design Choices

There are hundreds of possible improvements...  A lot of them are very
useful:

 - Chunk Size, Statistics, Diagnostics, Memory Trimming, Snapshots - See arena-b
 - Generation Counter and Key reservation - See atomic-arena
 - Growable - See blink-alloc
 - Memory Paging and Arena Growth - See arena-allocator
 - Memory Reclamation from Individual Items - See drop-arena
 - Scoped Allocator, so you can restore memory in stages - See bump-scope
 - Memory Pools - See shared-arena
 - Boxed Allocations or Collections so you CAN use an arena with strings
      and vecs. See Rodeo and Bumpalo
 - Memory Layout Control, Rewinding, Thread-Local memory lakes, etc (See lake)
 - Detect Use after free - See arena-allocator

Bitena is the Simple, Fast, and Multi-threaded solution.

# What NOT to do:

❌ - Don't do this:
```ignore
     let v = arena.try_alloc("Hello".to_string())?;    <== Still allocates from the heap
```

✅ - Do this instead:
```ignore
     let v = bitena.try_alloc("Hello")?;   <==  Arena based READ ONLY str
```

✅ - Do this instead: allocate a Box in the Arena, the string data from the heap.
```ignore
     let v = bitena.try_alloc(Box("Hello".to_string()))?; <== St from heap, Box handles drop
```


❌ - Don't do this:  
```ignore
     let v = bitena.try_alloc(vec![42u32; 10])?;  <== Allocates data on the heap
```

✅ - Do this instead:
```ignore
     let v = bitena.try_alloc_slice(42u32, 10)?;   <==  Returns a 10 element MUTABLE slice
```

✅ - Do this instead, allocate a Box in the Arena, Box allocates/deallocates Vec from the heap.
```ignore
     let v = bitena.try_alloc(Box(vec![42u32; 10]))?;  <==  Vec on heap, Box handles drop
```
In both cases of the Don't do this, a fat pointer will be stored in the arena,
and memory for the data or string will be allocated and LEAKED on the heap. In 

## License
MIT

## Contributions

All contributions intentionally submitted for inclusion in this work by you will
be governed by the MIT License without consideration of any additional terms or
conditions. By contributing to this project, you agree to license your
contributions under the MIT License.

## Credits
Everyone who's been part of the Open Source Movement. Thank you.
Reverse allocations inspired by:
  https://fitzgen.com/2019/11/01/always-bump-downwards.html
