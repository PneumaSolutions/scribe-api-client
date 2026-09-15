//! The exception types the Python API raises, and the mapping from the core
//! crate's errors onto them.

use pyo3::{create_exception, exceptions::PyException, prelude::*};

use scribe_client_core::ScribeError;

create_exception!(scribe_client, ScribeApiError, PyException);

create_exception!(scribe_client, InvalidGrantError, ScribeApiError);

create_exception!(scribe_client, NotFoundError, ScribeApiError);

create_exception!(scribe_client, ForbiddenError, ScribeApiError);

create_exception!(scribe_client, NotTrashedError, ScribeApiError);

create_exception!(scribe_client, ConversionNotCompleteError, ScribeApiError);

create_exception!(scribe_client, ConversionInProgressError, ScribeApiError);

create_exception!(scribe_client, RateLimitedError, ScribeApiError);

create_exception!(scribe_client, NeedsPurchaseError, ScribeApiError);

create_exception!(scribe_client, PasswordRequiredError, ScribeApiError);

pub(crate) fn to_py_err(err: ScribeError) -> PyErr {
    match err {
        ScribeError::InvalidGrant { message } => InvalidGrantError::new_err(message),
        ScribeError::NotFound { message } => NotFoundError::new_err(message),
        ScribeError::Forbidden { message } => ForbiddenError::new_err(message),
        ScribeError::NotTrashed { message } => NotTrashedError::new_err(message),
        ScribeError::ConversionNotComplete { message } => {
            ConversionNotCompleteError::new_err(message)
        }
        ScribeError::ConversionInProgress { message } => {
            ConversionInProgressError::new_err(message)
        }
        ScribeError::RateLimited { message } => RateLimitedError::new_err(message),
        ScribeError::NeedsPurchase {
            message,
            purchase_url,
        } => NeedsPurchaseError::new_err(format!("{message} ({purchase_url})")),
        ScribeError::PasswordRequired { message } => PasswordRequiredError::new_err(message),
        other => ScribeApiError::new_err(other.to_string()),
    }
}
