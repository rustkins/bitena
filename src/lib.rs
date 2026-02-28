//! # Bitena
//!
//! A small, extremely fast, lock-free thread-safe arena bump allocator that can hand out multiple
//! mutable elements, structs, slices, or read-only &strs from a single pre-allocated block.
//!
//! ## What is an Arena?
//!
//! An arena allocator is a memory allocation strategy that pre-allocates a large block
//! of memory once, and can then hand out sub-allocations from that block sequentially.
//! Bitena is are much faster than normal memory allocations because:
//!
//!   - **Bulk Allocation**: The entire arena is allocated all at once
//!   - **No Fragmentation**: Memory is allocated sequentially
//!   - **Fast Bookkeeping**: No complex tracking/reallocations
//!   - **Simplified Deallocation**: The entire arena is freed simultaneously
//!
//! `Bitena` is special in that, because of its design, it is not subject
//! to the same use after free or overlapped memory bugs that are possible with
//! some other bump allocators.
//!
//! ## Quick Start
//!
//! Add the following to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! bitena = "0.1"
//! ```
//!
//! ```rust
//!   use bitena::Bitena;
//!   fn main() {
//!     let mut bitena = Bitena::new(1024).expect("Failed to allocate memory");
//!     let num = bitena.alloc(42u32);
//!     let stnum = format!("Num: {}", *num);
//!     let s = bitena.alloc_str(&stnum);
//!     println!("{}  {:?}", *num, s);
//!   }
//! ```
//!
//! # The API
//!
//! ## new(byte_capacity)
//! Allocate a new Arena with a specified capacity.
//!
//! ## alloc(item) or try_alloc(item)
//! Allocate an element or structure in the Arena
//!
//! ## alloc_slice(init_value, items) or try_alloc_slice(init_value, items)
//! Allocate a slice vector of elements
//!
//! ## alloc_str(&str) or try_alloc_str(&str)
//! Store a &str in the Arena
//!
//! ## reset()
//! Reset the arena. This requires that all allocations are vacated, and
//! re-initializes the Arena to it's brand new state.
//!
//! ## Tradeoffs
//!
//!   - Individual Items are not resizeable. Each element or item allocated from
//!     the arena is a fixed size. You need to individually Box<T> any items, 
//!     (Strings, Vecs, Fat Pointers, file handles, etc) to avoid leaking memory.
//!
//!   - The entire arena will be dropped in a single operation. Individual Drop
//!     operations will not be performed on the Arena's contents. This then will
//!     leak any memory separately allocated as with Strings and Vecs.
//!
//!   - **No item Reclamation**: Any unused allocations are stuck until
//!     the whole arena is dropped or reset().
//!
//!   - **Fixed Size**: The arena has a set fixed size that doesn't grow.
//!
//! # MIRI to the Rescue
//!
//! Miri detected attempted memory leaks with String, and Vec in our tests.
//!
//! ```ignore
//!   cargo +nightly miri run
//! ```
//!
//!
//! # Use Cases:
//!
//!  - **Long-lived Data**: Perform one alloc from the system, and break that into
//!    all the allocations your need for the life of your program
//!
//!  - **Short-lived Processing**: Temporary allocations for a process... encoding,
//!    parsing, compiling, translation, etc. All the memory can be reused with reset()
//!    or set or returned/deallocated at the end of processing.
//!
//!  - **Saving Space**: Many system allocation schemes allocate in page sized blocks
//!    so freed memory can be more efficiently managed for reallocation. Arena fills
//!    every byte it can given alignment requirements.
//!
//!
//! # Design Choices
//!
//! There are hundreds of possible improvements...  A lot of them are very
//! useful:
//!
//!  - Chunk Size, Statistics, Diagnostics, Memory Trimming, Snapshots - See arena-b
//!  - Generation Counter and Key reservation - See atomic-arena
//!  - Growable - See blink-alloc
//!  - Memory Paging and Arena Growth - See arena-allocator
//!  - Memory Reclamation from Individual Items - See drop-arena
//!  - Scoped Allocator, so you can restore memory in stages - See bump-scope
//!  - Memory Pools - See shared-arena
//!  - Boxed Allocations or Collections so you CAN use an arena with strings
//!       and vecs. See Rodeo and Bumpalo
//!  - Memory Layout Control, Rewinding, Thread-Local memory lakes, etc (See lake)
//!  - Detect Use after free - See arena-allocator
//!
//! Bitena is the Simple, Fast, and Multi-threaded solution.
//!
//! # What NOT to do:
//!
//! ❌ - Don't do this:
//! ```ignore
//!      let v = arena.try_alloc("Hello".to_string())?;    <== Still allocates from the heap
//! ```
//!
//! ✅ - Do this instead:
//! ```ignore
//!      let v = bitena.try_alloc("Hello")?;   <==  Arena based READ ONLY str
//! ```
//!
//! ✅ - Do this instead: allocate a Box in the Arena, the string data from the heap.
//! ```ignore
//!      let v = bitena.try_alloc(Box("Hello".to_string()))?; <== St from heap, Box handles drop
//! ```
//!
//!
//! ❌ - Don't do this:  
//! ```ignore
//!      let v = bitena.try_alloc(vec![42u32; 10])?;  <== Allocates data on the heap
//! ```
//!
//! ✅ - Do this instead:
//! ```ignore
//!      let v = bitena.try_alloc_slice(42u32, 10)?;   <==  Returns a 10 element MUTABLE slice
//! ```
//!
//! ✅ - Do this instead, allocate a Box in the Arena, Box allocates/deallocates Vec from the heap.
//! ```ignore
//!      let v = bitena.try_alloc(Box(vec![42u32; 10]))?;  <==  Vec on heap, Box handles drop
//! ```
//! In both cases of the Don't do this, a fat pointer will be stored in the arena,
//! and memory for the data or string will be allocated and LEAKED on the heap. In 
//!
//! ## License
//! MIT
//!
//! ## Contributions
//!
//! All contributions intentionally submitted for inclusion in this work by you will
//! be governed by the MIT License without consideration of any additional terms or
//! conditions. By contributing to this project, you agree to license your
//! contributions under the MIT License.
//!
//! ## Credits
//! Everyone who's been part of the Open Source Movement. Thank you.
//! Reverse allocations inspired by:
//!   https://fitzgen.com/2019/11/01/always-bump-downwards.html

use std::alloc::{Layout, dealloc};
use std::marker::PhantomData;
use std::mem;
use std::num::NonZero;
use std::ptr::{copy_nonoverlapping, NonNull};
use std::sync::atomic::{AtomicUsize, Ordering};

mod error;
pub use self::error::{Error, Result};


/// Bitena
///
/// A small, extremely fast, lock-free thread-safe arena bump allocator that can hand out multiple
/// mutable elements, structs, slices, or read-only &strs from a single pre-allocated block.
///
/// This crate is optimized for performance-critical applications
///
///  - FIXED SIZED ITEMS
///
///  - ALLOCATED ITEMS DON'T DROP
///
///  - FIXED SIZE ARENA
///
/// any element types that reserver memory or resources, file handles,
/// vecs, and strings will leak memory if allocated on Bitena because 
/// the arena drops in one operation, without dropping each individual
/// item as appropriate.
///
/// # Example
///
/// ```
/// use bitena::*;
///
/// fn main() -> Result<()> {
///     let bitena = Bitena::new(1024)?;
///     let slice = bitena.try_alloc_slice(0u32, 4)?;
///     for i in 0..4 {
///         slice[i] = i as u32;
///     }
///     println!("{:?}", slice);
///     Ok(())
/// }
/// ```
pub struct Bitena<'a> {
    buf: NonNull<u8>,
    end_byte_idx: AtomicUsize, // Allows for interior mutability without Mutex, RefCells, Arcs
    layout: Layout,            // Stores byte_capacity
    _marker: PhantomData<&'a ()>,
}

impl<'a> Bitena<'a> {
    /// Creates a new Arena with the specified byte capacity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bitena::*;
    ///
    /// fn main() -> Result<()> {
    ///     let bitena = Bitena::new(1024)?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn new(byte_capacity: usize) -> Result<Self> {
        assert!(byte_capacity > 0, "Capacity must be greater than zero.");

        let layout = Layout::from_size_align(byte_capacity, mem::align_of::<u8>())?;
        let buf = unsafe {
            let ptr = std::alloc::alloc(layout);
            if ptr.is_null() {
                return Err(Error::OutOfMemory);
            }
            ptr as *mut u8
        };
        Ok(Self {
            buf: NonNull::new(buf).ok_or(Error::PointerUnderflow)?,
            end_byte_idx: AtomicUsize::new(byte_capacity),
            layout,
            _marker: PhantomData,
        })
    }

    /// Allocates space for a single element and returns a mutable reference to it.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The arena has enough remaining memory
    /// - The reference is not used after the arena is reset or dropped
    ///
    /// # Example
    ///
    /// ```
    /// use bitena::*;
    ///
    /// fn main() -> Result<()> {
    ///     let bitena = Bitena::new(1024)?;
    ///     let value = bitena.try_alloc(42u32)?;
    ///     let num = bitena.try_alloc(-234i32)?;
    ///     *num = 100;
    ///     println!("{}", *num);
    ///     Ok(())
    /// }
    /// ```
    #[inline]
    pub fn alloc<T>(&self, val: T) -> &mut T {
        self.try_alloc(val)
            .unwrap_or_else(|e| panic!("Bitena Failed: {}", e))
    }

    pub fn try_alloc<T>(&self, val: T) -> Result<&mut T> {
        let sizet = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        debug_assert!(sizet > 0, "Can't alloc 0 bytes");
        debug_assert!(align.is_power_of_two(), "Alignment must be a power of two");

        unsafe {
            loop {
                let end_byte_idx = self.end_byte_idx.load(Ordering::Relaxed);
                let ptr_num = (self.buf.as_ptr().add(end_byte_idx as usize) as usize)
                    .checked_sub(sizet)
                    .ok_or(Error::PointerUnderflow)?;

                //let ptr = (ptr as usize & !(align - 1)) as *mut u8;  // Align Ptr pre-Miri
                let ptr = self
                    .buf
                    .with_addr(NonZero::new(ptr_num & !(align - 1)).ok_or(Error::PointerUnderflow)?)
                    .as_ptr() as *mut u8;

                if (ptr as usize) < self.buf.as_ptr() as usize {
                    return Err(Error::OutOfMemory);
                }
                let new_end_byte_idx =
                    (ptr as usize).saturating_sub(self.buf.as_ptr() as usize) as usize;

                if let Ok(_) = self.end_byte_idx.compare_exchange_weak(
                    end_byte_idx,     // Expected value
                    new_end_byte_idx, // New value
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    std::ptr::write(ptr as *mut T, val);
                    return Ok(&mut *(ptr as *mut T));
                }
            }
        }
    }

    /// Allocates space for a slice and returns a mutable slice reference.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The arena has enough remaining memory
    /// - The reference is not used after the arena is reset or dropped
    ///
    /// # Example
    ///
    /// ```
    /// use bitena::*;
    ///
    /// fn main() -> Result<()> {
    ///     let bitena = Bitena::new(1024)?;
    ///         let slice = bitena.try_alloc_slice(0u32, 4)?;
    ///         for i in 0..4 {
    ///             slice[i] = i as u32;
    ///         }
    ///         println!("{:?}", slice);
    ///     Ok(())
    /// }
    /// ```
    #[inline]
    pub fn alloc_slice<T>(&self, initial_value: T, len: usize) -> &mut [T] {
        self.try_alloc_slice(initial_value, len)
            .unwrap_or_else(|e| panic!("Bitena Failed: {}", e))
    }

    pub fn try_alloc_slice<T>(&self, initial_value: T, len: usize) -> Result<&mut [T]> {
        let sizet = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        debug_assert!(sizet > 0, "Can't alloc 0 bytes");
        debug_assert!(align.is_power_of_two(), "Alignment must be a power of two");

        // This performs a compare and exchange loop on atomicUsize for the end_byte_idx value...
        // Making this algorithm safe for multi-thread apps
        unsafe {
            loop {
                let end_byte_idx = self.end_byte_idx.load(Ordering::Relaxed);
                let ptr_num = (self.buf.as_ptr().add(end_byte_idx) as usize)
                    .checked_sub(len * sizet)
                    .ok_or(Error::PointerUnderflow)?;

                //let ptr = (ptr as usize & !(align - 1)) as *mut u8;  // Align Ptr pre-Miri
                let ptr = self
                    .buf
                    .with_addr(NonZero::new(ptr_num & !(align - 1)).ok_or(Error::PointerUnderflow)?)
                    .as_ptr() as *mut u8;

                if (ptr as *mut u8 as usize) < self.buf.as_ptr() as usize {
                    return Err(Error::OutOfMemory);
                }
                let new_end_byte_idx =
                    (ptr as usize).saturating_sub(self.buf.as_ptr() as usize) as usize;

                if let Ok(_) = self.end_byte_idx.compare_exchange_weak(
                    end_byte_idx,     // Expected value
                    new_end_byte_idx, // New value
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    // Initialize New Slice
                    if sizet == 1 {
                        // Bytes are VERY FAST to initialize
                        let byte_ptr = &initial_value as *const T as *const u8;
                        std::ptr::write_bytes(ptr, *byte_ptr, len * sizet);
                    } else if is_all_zeros(&initial_value) {
                        // Zeroed Memory is too
                        std::ptr::write_bytes(ptr, 0, len * sizet);
                    } else {
                        // Not so fast!!!
                        let initial_value_ptr = &initial_value as *const T as *const u8;
                        for i in 0..len {
                            copy_nonoverlapping(
                                initial_value_ptr,
                                (ptr as *mut u8).add(i * sizet),
                                sizet,
                            );
                        }
                    }
                    return Ok(std::slice::from_raw_parts_mut(ptr as *mut T, len));
                }
            }
        }
    }

    /// Allocates space for a str and returns a read-only reference, &str.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The arena has enough remaining memory
    /// - The reference is not used after the arena is reset or dropped
    ///
    /// # Example
    ///
    /// ```
    /// use bitena::*;
    ///
    /// fn main() -> Result<()> {
    ///     let bitena = Bitena::new(1024)?;
    ///         let num = bitena.try_alloc(42u32)?;
    ///         *num = 100;
    ///         let stnum = format!("Num: {}", *num);
    ///         let s = bitena.try_alloc_str(&stnum)?;
    ///         println!("{}  {:?}", *num, s);
    ///     Ok(())
    /// }
    /// ```
    #[inline]
    pub fn alloc_str(&self, st: &str) -> &str {
        self.try_alloc_str(st)
            .unwrap_or_else(|e| panic!("Bitena Failed: {}", e))
    }

    pub fn try_alloc_str(&self, st: &str) -> Result<&str> {
        let sizet = st.len();
        let align = std::mem::align_of::<u8>();
        if sizet == 0 {
            return Ok::<&str, Error>("");
        }
        debug_assert!(align.is_power_of_two(), "Alignment must be a power of two");

        unsafe {
            loop {
                let end_byte_idx = self.end_byte_idx.load(Ordering::Relaxed);
                let ptr_num = (self.buf.as_ptr().add(end_byte_idx as usize) as usize)
                    .checked_sub(sizet)
                    .ok_or(Error::PointerUnderflow)?;

                //let ptr = (ptr as usize & !(align - 1)) as *mut u8;  // Align Ptr pre-Miri
                let ptr = self
                    .buf
                    .with_addr(NonZero::new(ptr_num & !(align - 1)).ok_or(Error::PointerUnderflow)?)
                    .as_ptr() as *mut u8;

                if (ptr as usize) < self.buf.as_ptr() as usize {
                    return Err(Error::OutOfMemory);
                }
                let new_end_byte_idx =
                    (ptr as usize).saturating_sub(self.buf.as_ptr() as usize) as usize;

                if let Ok(_) = self.end_byte_idx.compare_exchange_weak(
                    end_byte_idx,     // Expected value
                    new_end_byte_idx, // New value
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    copy_nonoverlapping(st.as_ptr(), ptr, sizet);
                    // Unchecked is Ok since the bytes came from a valid str
                    return Ok(std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                        ptr, sizet,
                    )));
                }
            }
        }
    }

    /// Returns the number of bytes remaining in the arena.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bitena::*;
    ///
    /// fn main() {
    ///     let bitena = Bitena::new(1024).unwrap();
    ///     assert_eq!(bitena.remaining(), 1024);
    /// }
    /// ```
    #[inline]
    pub fn remaining(&self) -> usize {
        self.end_byte_idx.load(Ordering::Relaxed)
    }

    /// Resets the arena, making all previously allocated memory available again.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bitena::*;
    ///
    /// fn main() -> Result<()> {
    ///     let mut bitena = Bitena::new(1024)?;
    ///     let slice = bitena.try_alloc_slice(1u8, 100)?;
    ///     assert_eq!(bitena.remaining(), 924);
    ///     bitena.reset();
    ///     assert_eq!(bitena.remaining(), 1024);
    ///     Ok(())
    /// }
    /// ```
    pub fn reset(&mut self) {
        loop {
            let end_byte_idx = self.end_byte_idx.load(Ordering::Relaxed);

            if let Ok(_) = self.end_byte_idx.compare_exchange_weak(
                end_byte_idx,       // Expected value
                self.layout.size(), // New value
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                return ();
            }
        }
    }
}

impl Drop for Bitena<'_> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            dealloc(self.buf.as_ptr(), self.layout);
        }
    }
}

unsafe impl Send for Bitena<'_> {}
unsafe impl Sync for Bitena<'_> {}

/// Returns IF value is comprised of all zeros.
#[inline]
fn is_all_zeros<T>(value: &T) -> bool {
    let num_bytes = std::mem::size_of::<T>();
    unsafe {
        let ptr = value as *const T as *const u8;
        for i in 0..num_bytes {
            if *ptr.add(i) != 0 {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod test {
    use super::*;
    use sysinfo::{Pid, System};

    #[test]
    fn test_try_alignment() -> Result<()> {
        let bitena = Bitena::new(1024)?;

        let _a = bitena.alloc_slice(0i8, 1); // 1-byte aligned
        let _a = bitena.alloc_slice(0i8, 1); // 1-byte aligned
        let a = bitena.alloc_slice(0i8, 1); // 1-byte aligned
        assert_eq!(a.as_ptr() as usize % 1, 0);

        let b = bitena.alloc(0u16); // 2-byte aligned
        assert_eq!(b as *const u16 as usize % 2, 0);

        let c = bitena.alloc_slice(0i32, 1); // 4-byte aligned
        assert_eq!(c.as_ptr() as usize % 4, 0);

        let d = bitena.alloc_slice(0i64, 1); // 8-byte aligned
        assert_eq!(d.as_ptr() as usize % 8, 0);

        // try_
        let a = bitena.try_alloc_slice(0i8, 1)?; // 1-byte aligned
        assert_eq!(a.as_ptr() as usize % 1, 0);

        let b = bitena.try_alloc(0u16)?; // 2-byte aligned
        assert_eq!(b as *const u16 as usize % 2, 0);

        let c = bitena.try_alloc_slice(0i32, 1)?; // 4-byte aligned
        assert_eq!(c.as_ptr() as usize % 4, 0);

        let d = bitena.try_alloc_slice(0i64, 1)?; // 8-byte aligned
        assert_eq!(d.as_ptr() as usize % 8, 0);
        Ok(())
    }

    #[test]
    fn test_try_bitena() -> Result<()> {
        let mut bitena = Bitena::new(1024)?;

        assert_eq!(bitena.remaining(), 1024, "Bitena should report 1024 bytes");

        let u8_ptr: &mut u8 = bitena.alloc(41u8);
        assert_eq!(bitena.remaining(), 1023, "Bitena should report 1023 bytes");

        let u32_ptr: &mut u32 = bitena.alloc(42u32);
        assert_eq!(
            u32_ptr as *mut u32 as usize % 4,
            0,
            "Pointer should be aligned"
        );

        *u32_ptr += *u8_ptr as u32;

        let u8_ptr: &mut [u8] = bitena.alloc_slice(43u8, 5);
        assert_eq!(u8_ptr, vec![43u8, 43, 43, 43, 43]);

        bitena.reset();

        assert_eq!(bitena.remaining(), 1024, "Bitena should report 1024 bytes");

        let _u8_ptr: &mut u8 = bitena.alloc(44u8);
        assert_eq!(bitena.remaining(), 1023, "Bitena should report 1023 bytes");

        let u64_ptr = bitena.alloc_slice(0u64, 4);
        assert_eq!(
            u64_ptr.as_ptr() as usize % 8,
            0,
            "u64 pointer should be 8-byte aligned"
        );

        let _u8_ptr: &mut u8 = bitena.alloc(46u8);

        let u128_ptr = bitena.alloc_slice(0u128, 5);
        assert_eq!(
            u128_ptr.as_ptr() as usize % 16,
            0,
            "u128 pointer should be 8-byte aligned"
        );

        // try_ testing:
        bitena.reset();
        let u8_ptr: &mut u8 = bitena.try_alloc(41u8)?;
        assert_eq!(bitena.remaining(), 1023, "Bitena should report 1023 bytes");

        let u32_ptr: &mut u32 = bitena.try_alloc(42u32)?;
        assert_eq!(
            u32_ptr as *mut u32 as usize % 4,
            0,
            "Pointer should be aligned"
        );

        *u32_ptr += *u8_ptr as u32;

        let u8_ptr: &mut [u8] = bitena.try_alloc_slice(43u8, 5)?;
        assert_eq!(u8_ptr, vec![43u8, 43, 43, 43, 43]);

        bitena.reset();

        assert_eq!(bitena.remaining(), 1024, "Bitena should report 1024 bytes");

        let _u8_ptr: &mut u8 = bitena.try_alloc(44u8)?;
        assert_eq!(bitena.remaining(), 1023, "Bitena should report 1023 bytes");

        let st = bitena.try_alloc_str("Test")?;
        assert_eq!(bitena.remaining(), 1019, "Bitena should report 1023 bytes");
        assert_eq!(st, "Test");

        let st = bitena.try_alloc_str("")?;
        assert_eq!(bitena.remaining(), 1019, "Bitena should report 1023 bytes");
        assert_eq!(st, "");

        let u64_ptr = bitena.try_alloc_slice(0u64, 4)?;
        assert_eq!(
            u64_ptr.as_ptr() as usize % 8,
            0,
            "u64 pointer should be 8-byte aligned"
        );

        let _u8_ptr: &mut u8 = bitena.try_alloc(46u8)?;

        let u128_ptr = bitena.try_alloc_slice(0u128, 5)?;
        assert_eq!(
            u128_ptr.as_ptr() as usize % 16,
            0,
            "u128 pointer should be 8-byte aligned"
        );

        Ok(())
    }

    #[test]
    #[should_panic(expected = "Layout Error: invalid parameters to Layout::from_size_align")]
    fn test_failed_to_allocate_panic() {
        let _bitena = Bitena::new(usize::MAX).unwrap_or_else(|e| panic!("Bitena Failed: {}", e));
    }

    #[test]
    fn test_try_failed_to_allocate() -> Result<()> {
        assert!(matches!(Bitena::new(usize::MAX), Err(Error::Layout(_))));
        Ok(())
    }

    #[test]
    #[should_panic(expected = "Bitena Failed: Out of Memory")]
    fn test_out_of_memory_panic() {
        let bitena = Bitena::new(1024).unwrap_or_else(|e| panic!("Should work Arena Failed: {}", e));
        let _large_slice: &mut [u64] = bitena.alloc_slice(0u64, 150);
    }

    #[test]
    fn test_try_out_of_memory() -> Result<()> {
        let bitena = Bitena::new(1024)?;
        // Check that alloc_slice returns Err(Error::OutOfMemory)
        assert!(matches!(
            bitena.try_alloc_slice(0u64, 150),
            Err(Error::OutOfMemory)
        ));
        Ok(())
    }

    fn format_number(n: u64) -> String {
        let s = n.to_string();
        let mut result = String::new();
        let mut count = 0;

        for c in s.chars().rev() {
            if count == 3 {
                result.push(',');
                count = 0;
            }
            result.push(c);
            count += 1;
        }

        result.chars().rev().collect()
    }

    fn get_system_available_memory() -> u64 {
        let sys = System::new_all();
        sys.available_memory()
    }

    fn get_process_memory_usage() -> u64 {
        let sys = System::new_all();
        let pid = Pid::from(std::process::id() as usize); // Convert to sysinfo's Pid type
        sys.process(pid)
            .map(|process: &sysinfo::Process| process.memory())
            .unwrap_or(0)
    }

    fn test_lg_alloc(size: usize) -> Result<()> {
        let bitena = Bitena::new(size)?;
        let j = bitena.try_alloc_slice(0u8, size)?;
        j.fill(15u8);
        Ok(())
    }


    // Note, Miri fails the sysconf(_SC_CLK_TCK) call.
    #[cfg_attr(miri, cfg(miri_skip))]
    #[test]
    fn test_try_large_allocation() -> Result<()> {
        const TARGET_SIZE: usize = 2 * 1024 * 1024 * 1024; // 2GB
        const NUM_ALLOCS: usize = 1000;
        const ALLOC_SIZE: usize = TARGET_SIZE / NUM_ALLOCS;

        //let bitena = Bitena::new(TARGET_SIZE);
        let start_sys = get_system_available_memory();
        let start_proc = get_process_memory_usage();

        println!(
            "Available memory: {} bytes",
            format_number(get_system_available_memory())
        );
        println!(
            "Process memory usage: {} bytes",
            format_number(get_process_memory_usage())
        );

        for _ in 0..NUM_ALLOCS {
            test_lg_alloc(ALLOC_SIZE).unwrap();
        }

        let used_sys = start_sys.saturating_sub(get_system_available_memory());
        let used_proc = get_process_memory_usage().saturating_sub(start_proc);

        println!(
            "Available memory: {} bytes, used: {}",
            format_number(get_system_available_memory()),
            format_number(used_sys)
        );
        println!(
            "Process memory usage: {} bytes, Additional proc mem used: {}",
            format_number(get_process_memory_usage()),
            format_number(used_proc)
        );

        assert!(used_sys < 50_000_000); // Arbitrary 50 MB Limit
        assert!(used_proc < 10_000_000); // Arbitrary 10 MB Limit
        Ok(())
    }
}
