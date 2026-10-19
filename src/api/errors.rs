//! API A.3 errors keep their public CLI exit dictionary.
use crate::{
    agent::{AppError, Code},
    api::Error,
};
pub fn app_error(error: Error) -> anyhow::Error {
    let code = match error.code.exit_code() {
        2 => Code::Usage,
        3 => Code::Safety,
        4 => Code::Backend,
        5 => Code::Upstream,
        6 => Code::NotFound,
        7 => Code::Ambiguous,
        8 => Code::Timeout,
        _ => Code::Failure,
    };
    let mut value = AppError::new(code, error.message);
    if let Some(hint) = error.hint {
        value = value.hint(hint);
    }
    value.into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retryable_never_maps_to_1() {
        use crate::api::ErrorCode::*;
        for code in [EngineUnavailable, Busy, Upstream, Timeout] {
            let error = app_error(Error {
                code,
                message: "retry".into(),
                retryable: true,
                hint: None,
                details: None,
            });
            assert_ne!(crate::agent::classify(&error).0 as u8, 1);
        }
    }
}
