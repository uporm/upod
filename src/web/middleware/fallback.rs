use crate::core::code::Code;
use crate::web::r::R;
use axum::http::{Method, Uri};
use axum::response::IntoResponse;

pub async fn not_found(uri: Uri) -> impl IntoResponse {
    let r: R<()> = R {
        code: Code::NotFound.into(),
        message: format!("Route not found: {}", uri),
        data: None,
    };
    r
}

pub async fn method_not_allowed(uri: Uri, method: Method) -> impl IntoResponse {
    let r: R<()> = R {
        code: Code::MethodNotAllowed.into(),
        message: format!("Method {} not allowed for {}", method, uri),
        data: None,
    };
    r
}
