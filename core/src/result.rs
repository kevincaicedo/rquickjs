use crate::{qjs, Ctx, Object, StdResult, StdString, Type};

use std::{
    error::Error as StdError,
    ffi::{CString, FromBytesWithNulError, NulError},
    fmt::{Display, Formatter, Result as FmtResult},
    io::Error as IoError,
    ops::Range,
    panic,
    panic::UnwindSafe,
    str::{FromStr, Utf8Error},
    string::FromUtf8Error,
};

/// Result type used throught the library.
pub type Result<T> = StdResult<T, Error>;

/// Error type of the library.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Could not allocate memory
    /// This is generally only triggered when out of memory.
    Allocation,
    /// A module defined two exported values with the same name.
    DuplicateExports,
    /// Found a string with a internal null byte while converting
    /// to C string.
    InvalidString(NulError),
    /// Found a string with a internal null byte while converting
    /// to C string.
    InvalidCStr(FromBytesWithNulError),
    /// String from rquickjs was not UTF-8
    Utf8(Utf8Error),
    /// An io error
    Io(IoError),
    /// An exception raised by quickjs itself.
    /// The actual javascript value can be retrieved by calling `Ctx::catch`.
    ///
    /// When returned from a callback the javascript will continue to unwind with the current
    /// error.
    Exception,
    /// Error converting from javascript to a rust type.
    FromJs {
        from: &'static str,
        to: &'static str,
        message: Option<StdString>,
    },
    /// Error converting to javascript from a rust type.
    IntoJs {
        from: &'static str,
        to: &'static str,
        message: Option<StdString>,
    },
    /// Error matching of function arguments
    NumArgs {
        expected: Range<usize>,
        given: usize,
    },
    #[cfg(feature = "loader")]
    /// Error when resolving js module
    Resolving {
        base: StdString,
        name: StdString,
        message: Option<StdString>,
    },
    #[cfg(feature = "loader")]
    /// Error when loading js module
    Loading {
        name: StdString,
        message: Option<StdString>,
    },
    /// Error when restoring a Persistent in a runtime other than the original runtime.
    UnrelatedRuntime,
    /// An error from quickjs from which the specifics are unknown.
    /// Should eventually be removed as development progresses.
    Unknown,
}

impl Error {
    #[cfg(feature = "loader")]
    /// Create resolving error
    pub fn new_resolving<B, N>(base: B, name: N) -> Self
    where
        StdString: From<B> + From<N>,
    {
        Error::Resolving {
            base: base.into(),
            name: name.into(),
            message: None,
        }
    }

    #[cfg(feature = "loader")]
    /// Create resolving error with message
    pub fn new_resolving_message<B, N, M>(base: B, name: N, msg: M) -> Self
    where
        StdString: From<B> + From<N> + From<M>,
    {
        Error::Resolving {
            base: base.into(),
            name: name.into(),
            message: Some(msg.into()),
        }
    }

    #[cfg(feature = "loader")]
    /// Returns whether the error is a resolving error
    pub fn is_resolving(&self) -> bool {
        matches!(self, Error::Resolving { .. })
    }

    #[cfg(feature = "loader")]
    /// Create loading error
    pub fn new_loading<N>(name: N) -> Self
    where
        StdString: From<N>,
    {
        Error::Loading {
            name: name.into(),
            message: None,
        }
    }

    #[cfg(feature = "loader")]
    /// Create loading error
    pub fn new_loading_message<N, M>(name: N, msg: M) -> Self
    where
        StdString: From<N> + From<M>,
    {
        Error::Loading {
            name: name.into(),
            message: Some(msg.into()),
        }
    }

    #[cfg(feature = "loader")]
    /// Returns whether the error is a loading error
    pub fn is_loading(&self) -> bool {
        matches!(self, Error::Loading { .. })
    }

    /// Returns whether the error is a quickjs generated exception.
    pub fn is_exception(&self) -> bool {
        matches!(self, Error::Exception)
    }

    /// Create from JS conversion error
    pub fn new_from_js(from: &'static str, to: &'static str) -> Self {
        Error::FromJs {
            from,
            to,
            message: None,
        }
    }

    /// Create from JS conversion error with message
    pub fn new_from_js_message<M>(from: &'static str, to: &'static str, msg: M) -> Self
    where
        StdString: From<M>,
    {
        Error::FromJs {
            from,
            to,
            message: Some(msg.into()),
        }
    }

    /// Create into JS conversion error
    pub fn new_into_js(from: &'static str, to: &'static str) -> Self {
        Error::IntoJs {
            from,
            to,
            message: None,
        }
    }

    /// Create into JS conversion error with message
    pub fn new_into_js_message<M>(from: &'static str, to: &'static str, msg: M) -> Self
    where
        StdString: From<M>,
    {
        Error::IntoJs {
            from,
            to,
            message: Some(msg.into()),
        }
    }

    /// Returns whether the error is a from JS conversion error
    pub fn is_from_js(&self) -> bool {
        matches!(self, Self::FromJs { .. })
    }

    /// Returns whether the error is a from JS to JS type conversion error
    pub fn is_from_js_to_js(&self) -> bool {
        matches!(self, Self::FromJs { to, .. } if Type::from_str(to).is_ok())
    }

    /// Returns whether the error is an into JS conversion error
    pub fn is_into_js(&self) -> bool {
        matches!(self, Self::IntoJs { .. })
    }

    /// Create function args mismatch error
    pub fn new_num_args(expected: Range<usize>, given: usize) -> Self {
        Self::NumArgs { expected, given }
    }

    /// Return whether the error is an function args mismatch error
    pub fn is_num_args(&self) -> bool {
        matches!(self, Self::NumArgs { .. })
    }

    /// Optimized conversion to CString
    pub(crate) fn to_cstring(&self) -> CString {
        // stringify error with NUL at end
        let mut message = format!("{self}\0").into_bytes();

        message.pop(); // pop last NUL because CString add this later

        // TODO: Replace by `CString::from_vec_with_nul_unchecked` later when it will be stabilized
        unsafe { CString::from_vec_unchecked(message) }
    }

    /// Throw an exception
    pub(crate) fn throw(&self, ctx: Ctx) -> qjs::JSValue {
        use Error::*;
        match self {
            Exception => qjs::JS_EXCEPTION,
            Allocation => unsafe { qjs::JS_ThrowOutOfMemory(ctx.as_ptr()) },
            InvalidString(_) | Utf8(_) | FromJs { .. } | IntoJs { .. } | NumArgs { .. } => {
                let message = self.to_cstring();
                unsafe { qjs::JS_ThrowTypeError(ctx.as_ptr(), message.as_ptr()) }
            }
            #[cfg(feature = "loader")]
            Resolving { .. } | Loading { .. } => {
                let message = self.to_cstring();
                unsafe { qjs::JS_ThrowReferenceError(ctx.as_ptr(), message.as_ptr()) }
            }
            Unknown => {
                let message = self.to_cstring();
                unsafe { qjs::JS_ThrowInternalError(ctx.as_ptr(), message.as_ptr()) }
            }
            error => {
                unsafe {
                    let value = qjs::JS_NewError(ctx.as_ptr());
                    if qjs::JS_VALUE_GET_NORM_TAG(value) == qjs::JS_TAG_EXCEPTION {
                        //allocation error happened, can't raise error properly. just immediately
                        //return
                        return value;
                    }
                    let obj = Object::from_js_value(ctx, value);
                    match obj.set("message", error.to_string()) {
                        Ok(_) => {}
                        Err(Error::Exception) => return qjs::JS_EXCEPTION,
                        Err(e) => {
                            panic!("generated error while throwing error: {}", e);
                        }
                    }
                    return qjs::JS_Throw(ctx.as_ptr(), obj.into_js_value());
                }
            }
        }
    }
}

impl StdError for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        use Error::*;

        match self {
            Allocation => "Allocation failed while creating object".fmt(f)?,
            DuplicateExports => {
                "Tried to export two values with the same name from one module".fmt(f)?
            }
            InvalidString(error) => {
                "String contained internal null bytes: ".fmt(f)?;
                error.fmt(f)?;
            }
            InvalidCStr(error) => {
                "CStr didn't end in a null byte: ".fmt(f)?;
                error.fmt(f)?;
            }
            Utf8(error) => {
                "Conversion from string failed: ".fmt(f)?;
                error.fmt(f)?;
            }
            Unknown => "quickjs library created a unknown error".fmt(f)?,
            Exception => "quickjs generated an exception".fmt(f)?,
            FromJs { from, to, message } => {
                "Error converting from js '".fmt(f)?;
                from.fmt(f)?;
                "' into type '".fmt(f)?;
                to.fmt(f)?;
                "'".fmt(f)?;
                if let Some(message) = message {
                    if !message.is_empty() {
                        ": ".fmt(f)?;
                        message.fmt(f)?;
                    }
                }
            }
            IntoJs { from, to, message } => {
                "Error converting from '".fmt(f)?;
                from.fmt(f)?;
                "' into js '".fmt(f)?;
                to.fmt(f)?;
                "'".fmt(f)?;
                if let Some(message) = message {
                    if !message.is_empty() {
                        ": ".fmt(f)?;
                        message.fmt(f)?;
                    }
                }
            }
            NumArgs { expected, given } => {
                "Error calling function with ".fmt(f)?;
                given.fmt(f)?;
                " argument(s) while ".fmt(f)?;
                expected.start.fmt(f)?;
                "..".fmt(f)?;
                if expected.end < usize::MAX {
                    expected.end.fmt(f)?;
                }
                " expected".fmt(f)?;
            }
            #[cfg(feature = "loader")]
            Resolving {
                base,
                name,
                message,
            } => {
                "Error resolving module '".fmt(f)?;
                name.fmt(f)?;
                "' from '".fmt(f)?;
                base.fmt(f)?;
                "'".fmt(f)?;
                if let Some(message) = message {
                    if !message.is_empty() {
                        ": ".fmt(f)?;
                        message.fmt(f)?;
                    }
                }
            }
            #[cfg(feature = "loader")]
            Loading { name, message } => {
                "Error loading module '".fmt(f)?;
                name.fmt(f)?;
                "'".fmt(f)?;
                if let Some(message) = message {
                    if !message.is_empty() {
                        ": ".fmt(f)?;
                        message.fmt(f)?;
                    }
                }
            }
            Io(error) => {
                "IO Error: ".fmt(f)?;
                error.fmt(f)?;
            }
            UnrelatedRuntime => "Restoring Persistent in an unrelated runtime".fmt(f)?,
        }
        Ok(())
    }
}

macro_rules! from_impls {
    ($($type:ty => $variant:ident,)*) => {
        $(
            impl From<$type> for Error {
                fn from(error: $type) -> Self {
                    Error::$variant(error)
                }
            }
        )*
    };
}

from_impls! {
    NulError => InvalidString,
    FromBytesWithNulError => InvalidCStr,
    Utf8Error => Utf8,
    IoError => Io,
}

impl From<FromUtf8Error> for Error {
    fn from(error: FromUtf8Error) -> Self {
        Error::Utf8(error.utf8_error())
    }
}

impl<'js> Ctx<'js> {
    pub(crate) fn handle_panic<F>(self, f: F) -> qjs::JSValue
    where
        F: FnOnce() -> qjs::JSValue + UnwindSafe,
    {
        unsafe {
            match panic::catch_unwind(f) {
                Ok(x) => x,
                Err(e) => {
                    self.get_opaque().panic = Some(e);
                    qjs::JS_Throw(self.as_ptr(), qjs::JS_MKVAL(qjs::JS_TAG_EXCEPTION, 0))
                }
            }
        }
    }

    /// Handle possible exceptions in JSValue's and turn them into errors
    /// Will return the JSValue if it is not an exception
    ///
    /// # Safety
    /// Assumes to have ownership of the JSValue
    pub(crate) unsafe fn handle_exception(self, js_val: qjs::JSValue) -> Result<qjs::JSValue> {
        if qjs::JS_VALUE_GET_NORM_TAG(js_val) != qjs::JS_TAG_EXCEPTION {
            Ok(js_val)
        } else {
            if let Some(x) = self.get_opaque().panic.take() {
                panic::resume_unwind(x)
            }
            Err(Error::Exception)
        }
    }

    /// Returns Error::Exception if there is no existing panic,
    /// otherwise continues panicking.
    pub(crate) unsafe fn raise_exception(self) -> Error {
        if let Some(x) = self.get_opaque().panic.take() {
            panic::resume_unwind(x)
        }
        Error::Exception
    }
}
