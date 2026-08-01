use axum::{
    body::Body,
    extract::FromRequest,
    http::{header, Request, Response, StatusCode},
    response::IntoResponse,
};
use serde::{de::DeserializeOwned, Serialize};

pub struct Json<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = axum::extract::rejection::JsonRejection;

    async fn from_request(req: Request<Body>, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(val) = axum::Json::<T>::from_request(req, state).await?;
        Ok(Json(val))
    }
}

impl<T> IntoResponse for Json<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response<Body> {
        let mut json_bytes = match serde_json::to_vec(&self.0) {
            Ok(bytes) => bytes,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    err.to_string(),
                )
                    .into_response();
            }
        };

        json_bytes.push(b'\n');

        ([(header::CONTENT_TYPE, "application/json")], json_bytes).into_response()
    }
}
