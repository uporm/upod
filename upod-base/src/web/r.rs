use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;

use crate::core::code::Code;
use crate::web::error::WebError;
use rust_i18n::t;
use tracing::{debug, error};
use validator::ValidationErrors;

#[derive(Serialize)]
pub struct R<T: Serialize> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
}

impl<T: Serialize> R<T> {
    pub fn ok(data: T) -> Self {
        let code = Code::Ok.into();
        Self {
            code,
            message: translate(code, &vec![]),
            data: Some(data),
        }
    }

    pub fn err(err: WebError) -> Self {
        let (code, message) = map_err(err);
        Self {
            code,
            message,
            data: None,
        }
    }

    pub fn from<E>(result: Result<T, E>) -> Self
    where
        WebError: From<E>,
    {
        match result {
            Ok(data) => Self::ok(data),
            Err(err) => Self::err(WebError::from(err)),
        }
    }
}

impl<T: Serialize> From<T> for R<T> {
    fn from(data: T) -> Self {
        Self::ok(data)
    }
}

impl<T: Serialize> From<WebError> for R<T> {
    fn from(err: WebError) -> Self {
        Self::err(err)
    }
}

impl R<()> {
    pub fn void() -> Self {
        Self::ok(())
    }

    pub fn from_unit<E>(result: Result<(), E>) -> Self
    where
        WebError: From<E>,
    {
        match result {
            Ok(_) => Self::ok(()),
            Err(err) => Self::err(WebError::from(err)),
        }
    }
}

impl<T: Serialize> IntoResponse for R<T> {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from_u16(self.code as u16).unwrap_or(StatusCode::OK);
        (status, Json(self)).into_response()
    }
}

fn map_err(err: WebError) -> (i32, String) {
    match err {
        WebError::Val(err) => {
            debug!("{:?}", err);
            let msg = format_validation_errors(&err);
            (Code::IllegalParam.into(), msg)
        }
        WebError::Biz(code) => (code, translate(code, &vec![])),
        WebError::BizWithArgs(code, args) => (code, translate(code, &args)),
        _ => {
            error!("{:?}", err);
            (
                Code::InternalServerError.into(),
                translate(Code::InternalServerError.into(), &vec![]),
            )
        }
    }
}

fn format_validation_errors(err: &ValidationErrors) -> String {
    let mut msgs = Vec::new();
    for (field, errs) in err.field_errors() {
        for e in errs {
            let detail = match e.code.as_ref() {
                "required" => t!(Code::ValidationRequired.to_string()).to_string(),
                "length" => {
                    let min = e.params.get("min").map(|v| v.to_string());
                    let max = e.params.get("max").map(|v| v.to_string());
                    match (min, max) {
                        (Some(min), Some(max)) => {
                            t!(Code::ValidationLengthBetween.to_string(), min => min, max => max)
                                .to_string()
                        }
                        (Some(min), None) => {
                            t!(Code::ValidationLengthMin.to_string(), min => min).to_string()
                        }
                        (None, Some(max)) => {
                            t!(Code::ValidationLengthMax.to_string(), max => max).to_string()
                        }
                        _ => t!(Code::ValidationLengthInvalid.to_string()).to_string(),
                    }
                }
                "range" => {
                    let min = e.params.get("min").map(|v| v.to_string());
                    let max = e.params.get("max").map(|v| v.to_string());
                    match (min, max) {
                        (Some(min), Some(max)) => {
                            t!(Code::ValidationRangeBetween.to_string(), min => min, max => max)
                                .to_string()
                        }
                        (Some(min), None) => {
                            t!(Code::ValidationRangeMin.to_string(), min => min).to_string()
                        }
                        (None, Some(max)) => {
                            t!(Code::ValidationRangeMax.to_string(), max => max).to_string()
                        }
                        _ => t!(Code::ValidationRangeInvalid.to_string()).to_string(),
                    }
                }
                "email" => t!(Code::ValidationEmail.to_string()).to_string(),
                _ => e.message.clone().map(|m| m.to_string()).unwrap_or_else(|| {
                    t!(Code::ValidationUnknown.to_string(), code => e.code).to_string()
                }),
            };
            msgs.push(format!("{}: {}", field, detail));
        }
    }
    if msgs.is_empty() {
        translate(Code::IllegalParam.into(), &vec![])
    } else {
        msgs.join("; ")
    }
}

fn translate(code: i32, params: &Vec<(String, String)>) -> String {
    let key = code.to_string();
    // 使用 t! 宏进行翻译，如果有参数则进行替换
    // 注意：rust-i18n 的 t! 宏在运行时替换可能需要不同的方式，这里保持手动替换逻辑作为后备或增强
    // 但更标准的做法是尽可能利用 t! 的插值能力。
    // 由于 t! 宏是编译时展开，这里用 t!(&key) 可能只能拿到原始模板字符串，
    // 所以手动 replace 是正确的运行时处理动态 key 的方式。
    let mut message = t!(&key).to_string();
    for (k, v) in params {
        // 尝试替换 {key} 格式的占位符
        message = message.replace(&format!("{{{}}}", k), v);
        // 尝试替换 %{key} 格式的占位符（有些 i18n 库习惯）
        message = message.replace(&format!("%{{{}}}", k), v);
    }
    message
}

#[macro_export]
macro_rules! r {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(err) => return $crate::web::r::R::err(err.into()),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use validator::ValidationError;

    #[test]
    fn test_validation_messages() {
        // Set locale to en
        rust_i18n::set_locale("en");

        let mut errs = ValidationErrors::new();
        errs.add("field1", ValidationError::new("required"));

        let msg = format_validation_errors(&errs);
        assert!(msg.contains("is required"));

        // Set locale to zh
        rust_i18n::set_locale("zh");
        let msg_zh = format_validation_errors(&errs);
        assert!(msg_zh.contains("不能为空"));

        // Test length
        let mut errs_len = ValidationErrors::new();
        let mut err_len = ValidationError::new("length");
        err_len.add_param(std::borrow::Cow::from("min"), &10);
        err_len.add_param(std::borrow::Cow::from("max"), &20);
        errs_len.add("field2", err_len);

        rust_i18n::set_locale("en");
        let msg_len = format_validation_errors(&errs_len);
        assert!(msg_len.contains("length must be between 10 and 20"));

        rust_i18n::set_locale("zh");
        let msg_len_zh = format_validation_errors(&errs_len);
        assert!(msg_len_zh.contains("长度必须在 10 和 20 之间"));
    }
}
