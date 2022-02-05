#[cfg(feature = "_internal_c_ffi")]
use std::os::raw::c_void;
use std::mem::MaybeUninit;

pub struct SeaCow<'a, T> {
    inner: SeaCowInner<'a, T>,
}

unsafe impl<T: Send> Send for SeaCowInner<'_, T> {}
unsafe impl<T: Sync> Sync for SeaCowInner<'_, T> {}

unsafe impl<T: Send> Send for SeaCow<'_, *const T> {}
unsafe impl<T: Sync> Sync for SeaCow<'_, *const T> {}

impl<'a, T> SeaCow<'a, T> {
    #[inline]
    pub fn borrowed(data: &'a [T]) -> Self {
        Self {
            inner: SeaCowInner::Borrowed(data),
        }
    }

    #[inline]
    pub fn boxed(data: Box<[T]>) -> Self {
        Self {
            inner: SeaCowInner::Boxed(data),
        }
    }

    /// The pointer must be `malloc`-allocated
    #[inline]
    #[cfg(feature = "_internal_c_ffi")]
    pub unsafe fn c_owned(ptr: *mut T, len: usize, free_fn: unsafe extern fn(*mut c_void)) -> Self {
        debug_assert!(!ptr.is_null());
        debug_assert!(len > 0);

        Self {
            inner: SeaCowInner::Owned { ptr, len, free_fn },
        }
    }

    #[inline]
    #[cfg(feature = "_internal_c_ffi")]
    pub(crate) fn make_owned(&mut self, free_fn: unsafe extern fn(*mut c_void)) {
        if let SeaCowInner::Borrowed(slice) = self.inner {
            self.inner = SeaCowInner::Owned { ptr: slice.as_ptr() as *mut _, len: slice.len(), free_fn };
        }
    }
}

enum SeaCowInner<'a, T> {
    #[cfg(feature = "_internal_c_ffi")]
    Owned { ptr: *mut T, len: usize, free_fn: unsafe extern fn(*mut c_void) },
    Borrowed(&'a [T]),
    Boxed(Box<[T]>),
}

#[cfg(feature = "_internal_c_ffi")]
impl<'a, T> Drop for SeaCowInner<'a, T> {
    fn drop(&mut self) {
        if let Self::Owned { ptr, free_fn, .. } = self {
            unsafe {
                (free_fn)((*ptr).cast());
            }
        }
    }
}

impl<'a, T> SeaCow<'a, T> {
    pub fn as_slice(&self) -> &[T] {
        match &self.inner {
            #[cfg(feature = "_internal_c_ffi")]
            SeaCowInner::Owned { ptr, len, .. } => unsafe { std::slice::from_raw_parts(*ptr, *len) },
            SeaCowInner::Borrowed(a) => a,
            SeaCowInner::Boxed(x) => x,
        }
    }
}

pub(crate) struct RowBitmap<'a, T> {
    rows: &'a [*const T],
    width: usize,
}
unsafe impl<T: Send + Sync> Send for RowBitmap<'_, T> {}

pub(crate) struct RowBitmapMut<'a, T> {
    rows: MutCow<'a, [*mut T]>,
    width: usize,
}
unsafe impl<T: Send + Sync> Send for RowBitmapMut<'_, T> {}

impl<'a, T> RowBitmapMut<'a, MaybeUninit<T>> {
    #[inline]
    pub(crate) unsafe fn assume_init<'maybeowned>(&'maybeowned mut self) -> RowBitmap<'maybeowned, T> {
        RowBitmap {
            width: self.width,
            rows: std::mem::transmute::<&'maybeowned [*mut MaybeUninit<T>], &'maybeowned [*const T]>(self.rows.borrow()),
        }
    }
}

impl<'a, T> RowBitmap<'a, T> {
    pub fn rows(&self) -> impl Iterator<Item = &[T]> {
        let width = self.width;
        self.rows.iter().map(move |&row| {
            unsafe { std::slice::from_raw_parts(row, width) }
        })
    }
}

enum MutCow<'a, T: ?Sized> {
    Owned(Box<T>),
    #[allow(dead_code)] /// This is optional, for FFI only
    Borrowed(&'a mut T),
}

impl<'a, T: ?Sized> MutCow<'a, T> {
    pub fn borrow(&mut self) -> &mut T {
        match self {
            Self::Owned(a) => a,
            Self::Borrowed(a) => a,
        }
    }
}

impl<'a, T: Sync + Send + Copy + 'static> RowBitmapMut<'a, T> {
    #[inline]
    pub fn new_contiguous(data: &mut [T], width: usize) -> Self {
        Self {
            rows: MutCow::Owned(data.chunks_exact_mut(width).map(|r| r.as_mut_ptr()).collect()),
            width,
        }
    }

    /// Inner pointers must be valid for `'a` too, and at least `width` large each
    #[inline]
    #[cfg(feature = "_internal_c_ffi")]
    pub unsafe fn new(rows: &'a mut [*mut T], width: usize) -> Self {
        Self {
            rows: MutCow::Borrowed(rows),
            width,
        }
    }

    pub fn rows_mut(&mut self) -> impl Iterator<Item = &mut [T]> + Send {
        let width = self.width;
        // Rust is pessimistic about `*mut` pointers
        struct ItIsSync<T>(*mut T);
        unsafe impl<T: Send + Sync> Sync for ItIsSync<T> {}
        let send_slice = unsafe { std::mem::transmute::<&mut [*mut T], &mut [ItIsSync<T>]>(self.rows.borrow()) };
        send_slice.iter().map(move |row| {
            unsafe { std::slice::from_raw_parts_mut(row.0, width) }
        })
    }
}
