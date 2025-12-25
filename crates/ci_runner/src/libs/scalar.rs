//! Scalar UI for OpenAPI documentation

use actix_web::{HttpResponse, Responder, Result as ActixResult};

const SCALAR_HTML: &str = r#"
<!doctype html>
<html>
  <head>
    <title>CI Runner API Documentation</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <style>
      body {
        margin: 0;
      }
    </style>
  </head>
  <body>
    <script
      id="api-reference"
      type="application/json"
      data-configuration='{
        "theme": "default",
        "layout": "modern",
        "spec": {
          "url": "/api-docs/openapi.json"
        }
      }'
    ></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>
"#;

pub async fn scalar_ui() -> ActixResult<impl Responder> {
    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(SCALAR_HTML))
}

