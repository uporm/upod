use crate::core::code::Code;
use crate::web::r::R;
use axum::Json as AxumJson;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use rust_i18n::t;
use serde::de::DeserializeOwned;

pub struct Json<T>(pub T);

impl<S, T> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match AxumJson::<T>::from_request(req, state).await {
            Ok(value) => Ok(Json(value.0)),
            Err(rejection) => {
                let message = match rejection {
                    JsonRejection::JsonDataError(e) => format_serde_error(&e.body_text()),
                    JsonRejection::MissingJsonContentType(_) => t!(
                        Code::MissingHeader.to_string(),
                        field = "Content-Type: application/json"
                    )
                    .to_string(),
                    _ => t!(Code::IllegalParam.to_string(), field = "Unknown").to_string(),
                };

                let r: R<()> = R {
                    code: Code::IllegalParam.into(),
                    message,
                    data: None,
                };
                Err(r.into_response())
            }
        }
    }
}

fn format_serde_error(error_text: &str) -> String {
    if let Some(start) = error_text.find("missing field `") {
        let rest = &error_text[start + 15..];
        if let Some(end) = rest.find('`') {
            let field = &rest[..end];
            return t!(Code::MissingParam.to_string(), field = field).to_string();
        }
    }

    t!(Code::IllegalParam.to_string(), field = error_text).to_string()
}
