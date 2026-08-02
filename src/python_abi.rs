//! Minimal CPython 3.10+ stable-ABI boundary used by FullBleed's Python feature.
//!
//! This is intentionally a small, owned-reference-safe layer over the limited C API. It exists so
//! the engine does not need a binding generator or a third-party runtime crate. Public Python
//! signatures live in the shipped standard-library-only facade; this module owns only value
//! conversion, error propagation, GIL release, and the one low-level dispatch function.

#![allow(non_snake_case)]

use std::collections::{BTreeMap, HashMap};
use std::ffi::{CString, c_char, c_double, c_int, c_longlong, c_ulonglong, c_void};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::ptr;
use std::rc::Rc;

pub(crate) mod ffi {
    use super::*;

    #[repr(C)]
    pub(crate) struct PyObject {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub(crate) struct PyThreadState {
        _private: [u8; 0],
    }

    pub(crate) type PySsizeT = isize;
    pub(crate) type PyCFunction =
        unsafe extern "C" fn(*mut PyObject, *mut PyObject) -> *mut PyObject;

    #[repr(C)]
    pub(crate) struct PyMethodDef {
        pub(crate) ml_name: *const c_char,
        pub(crate) ml_meth: Option<PyCFunction>,
        pub(crate) ml_flags: c_int,
        pub(crate) ml_doc: *const c_char,
    }

    #[repr(C)]
    pub(crate) struct PyObjectHead {
        pub(crate) ob_refcnt: PySsizeT,
        pub(crate) ob_type: *mut c_void,
    }

    #[repr(C)]
    pub(crate) struct PyModuleDefBase {
        pub(crate) ob_base: PyObjectHead,
        pub(crate) m_init: *mut c_void,
        pub(crate) m_index: PySsizeT,
        pub(crate) m_copy: *mut PyObject,
    }

    #[repr(C)]
    pub(crate) struct PyModuleDefSlot {
        pub(crate) slot: c_int,
        pub(crate) value: *mut c_void,
    }

    #[repr(C)]
    pub(crate) struct PyModuleDef {
        pub(crate) m_base: PyModuleDefBase,
        pub(crate) m_name: *const c_char,
        pub(crate) m_doc: *const c_char,
        pub(crate) m_size: PySsizeT,
        pub(crate) m_methods: *mut PyMethodDef,
        pub(crate) m_slots: *mut PyModuleDefSlot,
        pub(crate) m_traverse: *mut c_void,
        pub(crate) m_clear: *mut c_void,
        pub(crate) m_free: *mut c_void,
    }

    pub(crate) const METH_VARARGS: c_int = 0x0001;
    pub(crate) const PY_MOD_EXEC: c_int = 2;
    pub(crate) const PY_MOD_MULTIPLE_INTERPRETERS: c_int = 3;
    pub(crate) const PY_MOD_PER_INTERPRETER_GIL_SUPPORTED: usize = 2;
    #[cfg_attr(windows, link(name = "python3"))]
    unsafe extern "C" {
        pub(crate) fn Py_IncRef(object: *mut PyObject);
        pub(crate) fn Py_DecRef(object: *mut PyObject);

        pub(crate) fn PyErr_Occurred() -> *mut PyObject;
        pub(crate) fn PyErr_Fetch(
            error_type: *mut *mut PyObject,
            error_value: *mut *mut PyObject,
            traceback: *mut *mut PyObject,
        );
        pub(crate) fn PyErr_Restore(
            error_type: *mut PyObject,
            error_value: *mut PyObject,
            traceback: *mut PyObject,
        );
        pub(crate) fn PyErr_SetString(error_type: *mut PyObject, message: *const c_char);
        pub(crate) static mut PyExc_ValueError: *mut PyObject;
        pub(crate) static mut PyExc_TypeError: *mut PyObject;
        pub(crate) static mut PyExc_RuntimeError: *mut PyObject;

        pub(crate) fn PyImport_ImportModule(name: *const c_char) -> *mut PyObject;
        pub(crate) fn Py_GetVersion() -> *const c_char;
        pub(crate) fn PyEval_SaveThread() -> *mut PyThreadState;
        pub(crate) fn PyEval_RestoreThread(state: *mut PyThreadState);

        pub(crate) fn PyObject_GetAttrString(
            object: *mut PyObject,
            name: *const c_char,
        ) -> *mut PyObject;
        pub(crate) fn PyObject_Call(
            callable: *mut PyObject,
            args: *mut PyObject,
            kwargs: *mut PyObject,
        ) -> *mut PyObject;
        pub(crate) fn PyObject_GetIter(object: *mut PyObject) -> *mut PyObject;
        pub(crate) fn PyIter_Next(iterator: *mut PyObject) -> *mut PyObject;
        pub(crate) fn PyObject_IsTrue(object: *mut PyObject) -> c_int;
        pub(crate) fn PyObject_IsInstance(object: *mut PyObject, class: *mut PyObject) -> c_int;

        pub(crate) fn PyTuple_New(size: PySsizeT) -> *mut PyObject;
        pub(crate) fn PyTuple_Size(tuple: *mut PyObject) -> PySsizeT;
        pub(crate) fn PyTuple_GetItem(tuple: *mut PyObject, index: PySsizeT) -> *mut PyObject;
        pub(crate) fn PyTuple_SetItem(
            tuple: *mut PyObject,
            index: PySsizeT,
            item: *mut PyObject,
        ) -> c_int;

        pub(crate) fn PySequence_Size(sequence: *mut PyObject) -> PySsizeT;
        pub(crate) fn PySequence_GetItem(sequence: *mut PyObject, index: PySsizeT)
        -> *mut PyObject;

        pub(crate) fn PyList_New(size: PySsizeT) -> *mut PyObject;
        pub(crate) fn PyList_Size(list: *mut PyObject) -> PySsizeT;
        pub(crate) fn PyList_GetItem(list: *mut PyObject, index: PySsizeT) -> *mut PyObject;
        pub(crate) fn PyList_Append(list: *mut PyObject, item: *mut PyObject) -> c_int;

        pub(crate) fn PyDict_New() -> *mut PyObject;
        pub(crate) fn PyDict_SetItem(
            dict: *mut PyObject,
            key: *mut PyObject,
            value: *mut PyObject,
        ) -> c_int;
        pub(crate) fn PyDict_GetItemWithError(
            dict: *mut PyObject,
            key: *mut PyObject,
        ) -> *mut PyObject;
        pub(crate) fn PyDict_Next(
            dict: *mut PyObject,
            position: *mut PySsizeT,
            key: *mut *mut PyObject,
            value: *mut *mut PyObject,
        ) -> c_int;

        pub(crate) fn PyUnicode_FromStringAndSize(
            value: *const c_char,
            size: PySsizeT,
        ) -> *mut PyObject;
        pub(crate) fn PyUnicode_AsUTF8AndSize(
            unicode: *mut PyObject,
            size: *mut PySsizeT,
        ) -> *const c_char;

        pub(crate) fn PyBytes_FromStringAndSize(
            value: *const c_char,
            size: PySsizeT,
        ) -> *mut PyObject;
        pub(crate) fn PyBytes_AsStringAndSize(
            bytes: *mut PyObject,
            value: *mut *mut c_char,
            size: *mut PySsizeT,
        ) -> c_int;

        pub(crate) fn PyBool_FromLong(value: c_int) -> *mut PyObject;
        pub(crate) fn PyLong_FromLongLong(value: c_longlong) -> *mut PyObject;
        pub(crate) fn PyLong_FromUnsignedLongLong(value: c_ulonglong) -> *mut PyObject;
        pub(crate) fn PyLong_AsLongLong(value: *mut PyObject) -> c_longlong;
        pub(crate) fn PyLong_AsUnsignedLongLong(value: *mut PyObject) -> c_ulonglong;
        pub(crate) fn PyFloat_FromDouble(value: c_double) -> *mut PyObject;
        pub(crate) fn PyFloat_AsDouble(value: *mut PyObject) -> c_double;

        pub(crate) fn PyCapsule_New(
            pointer: *mut c_void,
            name: *const c_char,
            destructor: Option<unsafe extern "C" fn(*mut PyObject)>,
        ) -> *mut PyObject;
        pub(crate) fn PyCapsule_GetPointer(
            capsule: *mut PyObject,
            name: *const c_char,
        ) -> *mut c_void;

        pub(crate) fn PyModuleDef_Init(definition: *mut PyModuleDef) -> *mut PyObject;
    }
}

#[derive(Debug)]
enum PyErrKind {
    Native {
        error_type: *mut ffi::PyObject,
        error_value: *mut ffi::PyObject,
        traceback: *mut ffi::PyObject,
    },
    ValueError(String),
    TypeError(String),
    RuntimeError(String),
}

#[derive(Debug)]
pub(crate) struct PyErr {
    kind: PyErrKind,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl PyErr {
    pub(crate) fn fetch() -> Self {
        let mut error_type = ptr::null_mut();
        let mut error_value = ptr::null_mut();
        let mut traceback = ptr::null_mut();
        unsafe {
            ffi::PyErr_Fetch(&mut error_type, &mut error_value, &mut traceback);
        }
        if error_type.is_null() {
            return Self::runtime_error("CPython operation failed without setting an exception");
        }
        Self {
            kind: PyErrKind::Native {
                error_type,
                error_value,
                traceback,
            },
            _not_send_or_sync: PhantomData,
        }
    }

    pub(crate) fn value_error(message: impl Into<String>) -> Self {
        Self {
            kind: PyErrKind::ValueError(message.into()),
            _not_send_or_sync: PhantomData,
        }
    }

    pub(crate) fn type_error(message: impl Into<String>) -> Self {
        Self {
            kind: PyErrKind::TypeError(message.into()),
            _not_send_or_sync: PhantomData,
        }
    }

    pub(crate) fn runtime_error(message: impl Into<String>) -> Self {
        Self {
            kind: PyErrKind::RuntimeError(message.into()),
            _not_send_or_sync: PhantomData,
        }
    }

    pub(crate) unsafe fn restore(mut self) {
        let kind = std::mem::replace(&mut self.kind, PyErrKind::RuntimeError(String::new()));
        match kind {
            PyErrKind::Native {
                error_type,
                error_value,
                traceback,
            } => unsafe { ffi::PyErr_Restore(error_type, error_value, traceback) },
            PyErrKind::ValueError(message) => unsafe {
                set_string_exception(ffi::PyExc_ValueError, &message)
            },
            PyErrKind::TypeError(message) => unsafe {
                set_string_exception(ffi::PyExc_TypeError, &message)
            },
            PyErrKind::RuntimeError(message) => unsafe {
                set_string_exception(ffi::PyExc_RuntimeError, &message)
            },
        }
        std::mem::forget(self);
    }
}

impl Drop for PyErr {
    fn drop(&mut self) {
        if let PyErrKind::Native {
            error_type,
            error_value,
            traceback,
        } = &mut self.kind
        {
            unsafe {
                decref_non_null(*error_type);
                decref_non_null(*error_value);
                decref_non_null(*traceback);
            }
            *error_type = ptr::null_mut();
            *error_value = ptr::null_mut();
            *traceback = ptr::null_mut();
        }
    }
}

unsafe fn set_string_exception(error_type: *mut ffi::PyObject, message: &str) {
    let cleaned = message.replace('\0', "\\0");
    let message = CString::new(cleaned).expect("NULs replaced");
    unsafe { ffi::PyErr_SetString(error_type, message.as_ptr()) };
}

unsafe fn decref_non_null(object: *mut ffi::PyObject) {
    if !object.is_null() {
        unsafe { ffi::Py_DecRef(object) };
    }
}

pub(crate) type PyResult<T> = Result<T, PyErr>;

pub(crate) struct PyValueError;

impl PyValueError {
    pub(crate) fn new_err(message: impl Into<String>) -> PyErr {
        PyErr::value_error(message)
    }
}

struct OwnedPyPtr {
    pointer: *mut ffi::PyObject,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl OwnedPyPtr {
    unsafe fn from_owned(pointer: *mut ffi::PyObject) -> PyResult<Self> {
        if pointer.is_null() {
            return Err(PyErr::fetch());
        }
        Ok(Self {
            pointer,
            _not_send_or_sync: PhantomData,
        })
    }

    unsafe fn from_borrowed(pointer: *mut ffi::PyObject) -> PyResult<Self> {
        if pointer.is_null() {
            return Err(PyErr::runtime_error(
                "unexpected null borrowed Python object",
            ));
        }
        unsafe { ffi::Py_IncRef(pointer) };
        Ok(Self {
            pointer,
            _not_send_or_sync: PhantomData,
        })
    }

    fn into_raw(mut self) -> *mut ffi::PyObject {
        let pointer = self.pointer;
        self.pointer = ptr::null_mut();
        pointer
    }
}

impl Clone for OwnedPyPtr {
    fn clone(&self) -> Self {
        unsafe { ffi::Py_IncRef(self.pointer) };
        Self {
            pointer: self.pointer,
            _not_send_or_sync: PhantomData,
        }
    }
}

impl Drop for OwnedPyPtr {
    fn drop(&mut self) {
        unsafe { decref_non_null(self.pointer) };
        self.pointer = ptr::null_mut();
    }
}

pub(crate) enum PyAny {}
pub(crate) enum PyBytes {}
pub(crate) enum PyDict {}
pub(crate) enum PyList {}

#[repr(transparent)]
pub(crate) struct Bound<'py, T> {
    owned: OwnedPyPtr,
    marker: PhantomData<(&'py (), T)>,
}

impl<T> Clone for Bound<'_, T> {
    fn clone(&self) -> Self {
        Self {
            owned: self.owned.clone(),
            marker: PhantomData,
        }
    }
}

impl<'py, T> Bound<'py, T> {
    pub(crate) unsafe fn from_owned_ptr(pointer: *mut ffi::PyObject) -> PyResult<Self> {
        Ok(Self {
            owned: unsafe { OwnedPyPtr::from_owned(pointer)? },
            marker: PhantomData,
        })
    }

    pub(crate) unsafe fn from_borrowed_ptr(pointer: *mut ffi::PyObject) -> PyResult<Self> {
        Ok(Self {
            owned: unsafe { OwnedPyPtr::from_borrowed(pointer)? },
            marker: PhantomData,
        })
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::PyObject {
        self.owned.pointer
    }

    pub(crate) fn unbind(self) -> Py<T> {
        Py {
            owned: self.owned,
            marker: PhantomData,
        }
    }
}

#[repr(transparent)]
pub(crate) struct Py<T> {
    owned: OwnedPyPtr,
    marker: PhantomData<T>,
}

impl<T> Clone for Py<T> {
    fn clone(&self) -> Self {
        Self {
            owned: self.owned.clone(),
            marker: PhantomData,
        }
    }
}

impl<T> Py<T> {
    pub(crate) fn bind<'py>(&'py self, _py: Python<'py>) -> &'py Bound<'py, T> {
        // `Bound` and `Py` are both one owned-pointer field plus zero-sized lifetime/type markers.
        unsafe { &*(self as *const Py<T> as *const Bound<'py, T>) }
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::PyObject {
        self.owned.pointer
    }

    pub(crate) fn into_any(self) -> PyObject {
        Py {
            owned: self.owned,
            marker: PhantomData,
        }
    }

    pub(crate) fn into_raw(self) -> *mut ffi::PyObject {
        self.owned.into_raw()
    }
}

pub(crate) type PyObject = Py<PyAny>;
pub(crate) type PyRef<'py, T> = &'py T;

impl<'py> Bound<'py, PyAny> {
    pub(crate) fn downcast<T: PythonType>(&self) -> PyResult<&Bound<'py, T>> {
        if !T::matches(self.as_ptr())? {
            return Err(PyErr::type_error(format!("expected {}", T::NAME)));
        }
        // The marker type is zero-sized; both views own the same GIL-bound reference.
        Ok(unsafe { &*(self as *const Bound<'py, PyAny> as *const Bound<'py, T>) })
    }

    pub(crate) fn extract<T: FromPyObject>(&self) -> PyResult<T> {
        T::extract(self)
    }

    pub(crate) fn is_none(&self) -> bool {
        none_object().is_ok_and(|none| self.as_ptr() == none.as_ptr())
    }

    pub(crate) fn getattr(&self, name: &str) -> PyResult<Bound<'py, PyAny>> {
        let name = c_string(name)?;
        let pointer = unsafe { ffi::PyObject_GetAttrString(self.as_ptr(), name.as_ptr()) };
        unsafe { Bound::from_owned_ptr(pointer) }
    }

    pub(crate) fn call_method0(&self, name: &str) -> PyResult<Bound<'py, PyAny>> {
        let callable = self.getattr(name)?;
        call_python(&callable, Vec::new())
    }

    pub(crate) fn call_method1<A: IntoPyValue>(
        &self,
        name: &str,
        args: (A,),
    ) -> PyResult<Bound<'py, PyAny>> {
        let callable = self.getattr(name)?;
        call_python(&callable, vec![args.0.into_py_value()?])
    }

    pub(crate) fn try_iter(&self) -> PyResult<PyIterator<'py>> {
        let pointer = unsafe { ffi::PyObject_GetIter(self.as_ptr()) };
        Ok(PyIterator {
            iterator: unsafe { Bound::from_owned_ptr(pointer)? },
        })
    }
}

impl Bound<'_, PyBytes> {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        let mut pointer = ptr::null_mut();
        let mut size = 0;
        let status =
            unsafe { ffi::PyBytes_AsStringAndSize(self.as_ptr(), &mut pointer, &mut size) };
        debug_assert_eq!(status, 0);
        if status != 0 || pointer.is_null() || size <= 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size as usize) }
    }
}

pub(crate) struct PyIterator<'py> {
    iterator: Bound<'py, PyAny>,
}

impl<'py> Iterator for PyIterator<'py> {
    type Item = PyResult<Bound<'py, PyAny>>;

    fn next(&mut self) -> Option<Self::Item> {
        let pointer = unsafe { ffi::PyIter_Next(self.iterator.as_ptr()) };
        if !pointer.is_null() {
            return Some(unsafe { Bound::from_owned_ptr(pointer) });
        }
        if unsafe { ffi::PyErr_Occurred() }.is_null() {
            None
        } else {
            Some(Err(PyErr::fetch()))
        }
    }
}

pub(crate) struct PyListIterator<'py> {
    list: &'py Bound<'py, PyList>,
    index: isize,
    length: isize,
}

impl<'py> Iterator for PyListIterator<'py> {
    type Item = Bound<'py, PyAny>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.length {
            return None;
        }
        let pointer = unsafe { ffi::PyList_GetItem(self.list.as_ptr(), self.index) };
        self.index += 1;
        unsafe { Bound::from_borrowed_ptr(pointer).ok() }
    }
}

impl Bound<'_, PyList> {
    pub(crate) fn append<V: IntoPyValue>(&self, value: V) -> PyResult<()> {
        let value = value.into_py_value()?;
        let status = unsafe { ffi::PyList_Append(self.as_ptr(), value.as_ptr()) };
        if status != 0 {
            return Err(PyErr::fetch());
        }
        Ok(())
    }

    pub(crate) fn iter(&self) -> PyListIterator<'_> {
        let length = unsafe { ffi::PyList_Size(self.as_ptr()) }.max(0);
        PyListIterator {
            list: self,
            index: 0,
            length,
        }
    }

    pub(crate) fn len(&self) -> usize {
        unsafe { ffi::PyList_Size(self.as_ptr()) }.max(0) as usize
    }
}

pub(crate) struct PyDictIterator<'py> {
    dict: &'py Bound<'py, PyDict>,
    position: isize,
}

impl<'py> Iterator for PyDictIterator<'py> {
    type Item = (Bound<'py, PyAny>, Bound<'py, PyAny>);

    fn next(&mut self) -> Option<Self::Item> {
        let mut key = ptr::null_mut();
        let mut value = ptr::null_mut();
        let found = unsafe {
            ffi::PyDict_Next(self.dict.as_ptr(), &mut self.position, &mut key, &mut value)
        };
        if found == 0 {
            return None;
        }
        let key = unsafe { Bound::from_borrowed_ptr(key).ok()? };
        let value = unsafe { Bound::from_borrowed_ptr(value).ok()? };
        Some((key, value))
    }
}

impl<'py> Bound<'py, PyDict> {
    pub(crate) fn set_item<K: IntoPyValue, V: IntoPyValue>(
        &self,
        key: K,
        value: V,
    ) -> PyResult<()> {
        let key = key.into_py_value()?;
        let value = value.into_py_value()?;
        let status = unsafe { ffi::PyDict_SetItem(self.as_ptr(), key.as_ptr(), value.as_ptr()) };
        if status != 0 {
            return Err(PyErr::fetch());
        }
        Ok(())
    }

    pub(crate) fn get_item<K: IntoPyValue>(&self, key: K) -> PyResult<Option<Bound<'py, PyAny>>> {
        let key = key.into_py_value()?;
        let pointer = unsafe { ffi::PyDict_GetItemWithError(self.as_ptr(), key.as_ptr()) };
        if pointer.is_null() {
            if unsafe { ffi::PyErr_Occurred() }.is_null() {
                return Ok(None);
            }
            return Err(PyErr::fetch());
        }
        Ok(Some(unsafe { Bound::from_borrowed_ptr(pointer)? }))
    }

    pub(crate) fn iter(&self) -> PyDictIterator<'_> {
        PyDictIterator {
            dict: self,
            position: 0,
        }
    }
}

pub(crate) trait PythonType {
    const NAME: &'static str;
    fn matches(object: *mut ffi::PyObject) -> PyResult<bool>;
}

impl PythonType for PyAny {
    const NAME: &'static str = "object";
    fn matches(_object: *mut ffi::PyObject) -> PyResult<bool> {
        Ok(true)
    }
}

macro_rules! builtin_python_type {
    ($type:ty, $name:literal) => {
        impl PythonType for $type {
            const NAME: &'static str = $name;
            fn matches(object: *mut ffi::PyObject) -> PyResult<bool> {
                is_builtin_instance(object, $name)
            }
        }
    };
}

builtin_python_type!(PyBytes, "bytes");
builtin_python_type!(PyDict, "dict");
builtin_python_type!(PyList, "list");

#[derive(Clone, Copy)]
pub(crate) struct Python<'py> {
    marker: PhantomData<(&'py (), Rc<()>)>,
}

impl Python<'_> {
    pub(crate) fn with_gil<F, R>(function: F) -> R
    where
        F: for<'py> FnOnce(Python<'py>) -> R,
    {
        function(Python {
            marker: PhantomData,
        })
    }

    pub(crate) fn allow_threads<F, R>(self, function: F) -> R
    where
        F: FnOnce() -> R,
    {
        let state = unsafe { ffi::PyEval_SaveThread() };
        let result = catch_unwind(AssertUnwindSafe(function));
        unsafe { ffi::PyEval_RestoreThread(state) };
        match result {
            Ok(value) => value,
            Err(payload) => resume_unwind(payload),
        }
    }

    pub(crate) fn import(self, name: &str) -> PyResult<Bound<'_, PyAny>> {
        let name = c_string(name)?;
        let pointer = unsafe { ffi::PyImport_ImportModule(name.as_ptr()) };
        unsafe { Bound::from_owned_ptr(pointer) }
    }

    pub(crate) fn None(self) -> PyObject {
        none_object().expect("builtins.None must exist")
    }
}

impl PyBytes {
    pub(crate) fn new<'py>(_py: Python<'py>, bytes: &[u8]) -> Bound<'py, PyBytes> {
        let pointer =
            unsafe { ffi::PyBytes_FromStringAndSize(bytes.as_ptr().cast(), bytes.len() as isize) };
        unsafe { Bound::from_owned_ptr(pointer) }.expect("PyBytes allocation failed")
    }
}

impl PyDict {
    pub(crate) fn new<'py>(_py: Python<'py>) -> Bound<'py, PyDict> {
        unsafe { Bound::from_owned_ptr(ffi::PyDict_New()) }.expect("PyDict allocation failed")
    }
}

impl PyList {
    pub(crate) fn empty<'py>(_py: Python<'py>) -> Bound<'py, PyList> {
        unsafe { Bound::from_owned_ptr(ffi::PyList_New(0)) }.expect("PyList allocation failed")
    }

    pub(crate) fn new<'py, I, V>(_py: Python<'py>, values: I) -> PyResult<Bound<'py, PyList>>
    where
        I: IntoIterator<Item = V>,
        V: IntoPyValue,
    {
        let list = unsafe { Bound::from_owned_ptr(ffi::PyList_New(0)) }?;
        for value in values {
            list.append(value)?;
        }
        Ok(list)
    }
}

pub(crate) trait IntoPyValue {
    fn into_py_value(self) -> PyResult<PyObject>;
}

impl<T> IntoPyValue for Py<T> {
    fn into_py_value(self) -> PyResult<PyObject> {
        Ok(self.into_any())
    }
}

impl<T> IntoPyValue for Bound<'_, T> {
    fn into_py_value(self) -> PyResult<PyObject> {
        Ok(self.unbind().into_any())
    }
}

impl<T> IntoPyValue for &T
where
    T: Clone + IntoPyValue,
{
    fn into_py_value(self) -> PyResult<PyObject> {
        self.clone().into_py_value()
    }
}

impl IntoPyValue for &str {
    fn into_py_value(self) -> PyResult<PyObject> {
        string_to_py(self)
    }
}

impl IntoPyValue for String {
    fn into_py_value(self) -> PyResult<PyObject> {
        string_to_py(&self)
    }
}

impl IntoPyValue for bool {
    fn into_py_value(self) -> PyResult<PyObject> {
        unsafe { Bound::from_owned_ptr(ffi::PyBool_FromLong(i32::from(self))) }.map(Bound::unbind)
    }
}

macro_rules! signed_into_py {
    ($($type:ty),+ $(,)?) => {$(
        impl IntoPyValue for $type {
            fn into_py_value(self) -> PyResult<PyObject> {
                unsafe { Bound::from_owned_ptr(ffi::PyLong_FromLongLong(self as i64)) }
                    .map(Bound::unbind)
            }
        }
    )+};
}

macro_rules! unsigned_into_py {
    ($($type:ty),+ $(,)?) => {$(
        impl IntoPyValue for $type {
            fn into_py_value(self) -> PyResult<PyObject> {
                unsafe { Bound::from_owned_ptr(ffi::PyLong_FromUnsignedLongLong(self as u64)) }
                    .map(Bound::unbind)
            }
        }
    )+};
}

signed_into_py!(i8, i16, i32, i64, isize);
unsigned_into_py!(u8, u16, u32, u64, usize);

impl IntoPyValue for f32 {
    fn into_py_value(self) -> PyResult<PyObject> {
        (self as f64).into_py_value()
    }
}

impl IntoPyValue for f64 {
    fn into_py_value(self) -> PyResult<PyObject> {
        unsafe { Bound::from_owned_ptr(ffi::PyFloat_FromDouble(self)) }.map(Bound::unbind)
    }
}

impl<T: IntoPyValue> IntoPyValue for Option<T> {
    fn into_py_value(self) -> PyResult<PyObject> {
        match self {
            Some(value) => value.into_py_value(),
            None => Ok(Python::with_gil(|py| py.None())),
        }
    }
}

impl<T: IntoPyValue> IntoPyValue for Vec<T> {
    fn into_py_value(self) -> PyResult<PyObject> {
        Python::with_gil(|py| PyList::new(py, self).map(|value| value.unbind().into_any()))
    }
}

impl<T> IntoPyValue for &[T]
where
    T: Clone + IntoPyValue,
{
    fn into_py_value(self) -> PyResult<PyObject> {
        self.to_vec().into_py_value()
    }
}

impl<T: IntoPyValue> IntoPyValue for HashMap<String, T> {
    fn into_py_value(self) -> PyResult<PyObject> {
        map_to_py(self)
    }
}

impl<T: IntoPyValue> IntoPyValue for BTreeMap<String, T> {
    fn into_py_value(self) -> PyResult<PyObject> {
        map_to_py(self)
    }
}

impl<A: IntoPyValue, B: IntoPyValue> IntoPyValue for (A, B) {
    fn into_py_value(self) -> PyResult<PyObject> {
        tuple_to_py(vec![self.0.into_py_value()?, self.1.into_py_value()?])
    }
}

impl<A: IntoPyValue, B: IntoPyValue, C: IntoPyValue> IntoPyValue for (A, B, C) {
    fn into_py_value(self) -> PyResult<PyObject> {
        tuple_to_py(vec![
            self.0.into_py_value()?,
            self.1.into_py_value()?,
            self.2.into_py_value()?,
        ])
    }
}

impl<A: IntoPyValue, B: IntoPyValue, C: IntoPyValue, D: IntoPyValue> IntoPyValue for (A, B, C, D) {
    fn into_py_value(self) -> PyResult<PyObject> {
        tuple_to_py(vec![
            self.0.into_py_value()?,
            self.1.into_py_value()?,
            self.2.into_py_value()?,
            self.3.into_py_value()?,
        ])
    }
}

impl IntoPyValue for () {
    fn into_py_value(self) -> PyResult<PyObject> {
        Ok(Python::with_gil(|py| py.None()))
    }
}

fn map_to_py<I, V>(values: I) -> PyResult<PyObject>
where
    I: IntoIterator<Item = (String, V)>,
    V: IntoPyValue,
{
    Python::with_gil(|py| {
        let dict = PyDict::new(py);
        for (key, value) in values {
            dict.set_item(key, value)?;
        }
        Ok(dict.unbind().into_any())
    })
}

fn tuple_to_py(values: Vec<PyObject>) -> PyResult<PyObject> {
    let tuple = unsafe { Bound::<PyAny>::from_owned_ptr(ffi::PyTuple_New(values.len() as isize))? };
    for (index, value) in values.into_iter().enumerate() {
        let status =
            unsafe { ffi::PyTuple_SetItem(tuple.as_ptr(), index as isize, value.into_raw()) };
        if status != 0 {
            return Err(PyErr::fetch());
        }
    }
    Ok(tuple.unbind())
}

pub(crate) trait FromPyObject: Sized {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self>;
}

impl FromPyObject for PyObject {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(value.clone().unbind())
    }
}

impl FromPyObject for String {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut size = 0;
        let pointer = unsafe { ffi::PyUnicode_AsUTF8AndSize(value.as_ptr(), &mut size) };
        if pointer.is_null() {
            return Err(PyErr::fetch());
        }
        let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size as usize) };
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| PyErr::value_error("Python string is not valid UTF-8"))
    }
}

impl FromPyObject for bool {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if !is_builtin_instance(value.as_ptr(), "bool")? {
            return Err(PyErr::type_error("expected bool"));
        }
        match unsafe { ffi::PyObject_IsTrue(value.as_ptr()) } {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(PyErr::fetch()),
        }
    }
}

macro_rules! signed_from_py {
    ($($type:ty),+ $(,)?) => {$(
        impl FromPyObject for $type {
            fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
                let raw = unsafe { ffi::PyLong_AsLongLong(value.as_ptr()) };
                if raw == -1 && !unsafe { ffi::PyErr_Occurred() }.is_null() {
                    return Err(PyErr::fetch());
                }
                <$type>::try_from(raw).map_err(|_| PyErr::value_error("integer out of range"))
            }
        }
    )+};
}

macro_rules! unsigned_from_py {
    ($($type:ty),+ $(,)?) => {$(
        impl FromPyObject for $type {
            fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
                let raw = unsafe { ffi::PyLong_AsUnsignedLongLong(value.as_ptr()) };
                if raw == u64::MAX && !unsafe { ffi::PyErr_Occurred() }.is_null() {
                    return Err(PyErr::fetch());
                }
                <$type>::try_from(raw).map_err(|_| PyErr::value_error("integer out of range"))
            }
        }
    )+};
}

signed_from_py!(i8, i16, i32, i64, isize);
unsigned_from_py!(u8, u16, u32, u64, usize);

impl FromPyObject for f32 {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        f64::extract(value).map(|value| value as f32)
    }
}

impl FromPyObject for f64 {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let raw = unsafe { ffi::PyFloat_AsDouble(value.as_ptr()) };
        if raw == -1.0 && !unsafe { ffi::PyErr_Occurred() }.is_null() {
            return Err(PyErr::fetch());
        }
        Ok(raw)
    }
}

impl<T: FromPyObject> FromPyObject for Vec<T> {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut result = Vec::new();
        for item in value.try_iter()? {
            result.push(T::extract(&item?)?);
        }
        Ok(result)
    }
}

impl<T: FromPyObject> FromPyObject for HashMap<String, T> {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let dict = value.downcast::<PyDict>()?;
        let mut result = HashMap::new();
        for (key, value) in dict.iter() {
            result.insert(String::extract(&key)?, T::extract(&value)?);
        }
        Ok(result)
    }
}

impl<A: FromPyObject, B: FromPyObject> FromPyObject for (A, B) {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        sequence_length(value, 2)?;
        Ok((
            sequence_item(value, 0)?.extract()?,
            sequence_item(value, 1)?.extract()?,
        ))
    }
}

impl<A: FromPyObject, B: FromPyObject, C: FromPyObject, D: FromPyObject, E: FromPyObject>
    FromPyObject for (A, B, C, D, E)
{
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        sequence_length(value, 5)?;
        Ok((
            sequence_item(value, 0)?.extract()?,
            sequence_item(value, 1)?.extract()?,
            sequence_item(value, 2)?.extract()?,
            sequence_item(value, 3)?.extract()?,
            sequence_item(value, 4)?.extract()?,
        ))
    }
}

pub(crate) fn sequence_length(value: &Bound<'_, PyAny>, expected: isize) -> PyResult<()> {
    let length = unsafe { ffi::PySequence_Size(value.as_ptr()) };
    if length < 0 {
        return Err(PyErr::fetch());
    }
    if length != expected {
        return Err(PyErr::value_error(format!(
            "expected a {expected}-item sequence, got {length} items"
        )));
    }
    Ok(())
}

pub(crate) fn sequence_item<'py>(
    value: &Bound<'py, PyAny>,
    index: isize,
) -> PyResult<Bound<'py, PyAny>> {
    let pointer = unsafe { ffi::PySequence_GetItem(value.as_ptr(), index) };
    unsafe { Bound::from_owned_ptr(pointer) }
}

fn string_to_py(value: &str) -> PyResult<PyObject> {
    let pointer =
        unsafe { ffi::PyUnicode_FromStringAndSize(value.as_ptr().cast(), value.len() as isize) };
    unsafe { Bound::from_owned_ptr(pointer) }.map(Bound::unbind)
}

fn call_python<'py>(
    callable: &Bound<'py, PyAny>,
    args: Vec<PyObject>,
) -> PyResult<Bound<'py, PyAny>> {
    let tuple = tuple_to_py(args)?;
    let pointer = unsafe { ffi::PyObject_Call(callable.as_ptr(), tuple.as_ptr(), ptr::null_mut()) };
    unsafe { Bound::from_owned_ptr(pointer) }
}

fn c_string(value: &str) -> PyResult<CString> {
    CString::new(value.replace('\0', "\\0"))
        .map_err(|_| PyErr::value_error("string contains a NUL byte"))
}

fn builtin_object(name: &str) -> PyResult<PyObject> {
    Python::with_gil(|py| py.import("builtins")?.getattr(name).map(Bound::unbind))
}

fn none_object() -> PyResult<PyObject> {
    builtin_object("None")
}

fn is_builtin_instance(object: *mut ffi::PyObject, name: &str) -> PyResult<bool> {
    let class = builtin_object(name)?;
    match unsafe { ffi::PyObject_IsInstance(object, class.as_ptr()) } {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(PyErr::fetch()),
    }
}

pub(crate) unsafe fn tuple_argument<'py>(
    args: *mut ffi::PyObject,
    index: isize,
) -> PyResult<Bound<'py, PyAny>> {
    let pointer = unsafe { ffi::PyTuple_GetItem(args, index) };
    unsafe { Bound::from_borrowed_ptr(pointer) }
}

pub(crate) unsafe fn tuple_argument_count(args: *mut ffi::PyObject) -> PyResult<isize> {
    let count = unsafe { ffi::PyTuple_Size(args) };
    if count < 0 {
        Err(PyErr::fetch())
    } else {
        Ok(count)
    }
}

pub(crate) unsafe fn result_to_raw<T: IntoPyValue>(result: PyResult<T>) -> *mut ffi::PyObject {
    match result.and_then(IntoPyValue::into_py_value) {
        Ok(value) => value.into_raw(),
        Err(error) => {
            unsafe { error.restore() };
            ptr::null_mut()
        }
    }
}
