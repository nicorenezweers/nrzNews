//! `/mcp`: der MCP-Server (siehe `crate::mcp`) über Streamable HTTP. Jede Anfrage ist ein
//! POST mit einer JSON-RPC-Nachricht; die Antwort kommt direkt als `application/json` (kein
//! SSE-Strom, keine Sitzungen – der Server stellt keine eigenen Anfragen). Anmeldung mit einem
//! persönlichen API-Token: `Authorization: Bearer nrznews_…` (Einstellungen → Konto).

use crate::auth::guards::is_cross_origin;
use crate::mcp;
use crate::services::tokens;
use rocket::http::{ContentType, Header, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::{self, Responder, Response};
use rocket::{get, post, routes, Route, State};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::io::Cursor;

pub fn routes() -> Vec<Route> {
    routes![post_mcp, get_mcp]
}

/// Was der Aufrufer mitbringt: Token (falls vorhanden) und ob die Anfrage von einer fremden
/// Webseite kommt (Schutz gegen DNS-Rebinding, wie die MCP-Spezifikation verlangt).
pub struct Caller {
    token: Option<String>,
    cross_origin: bool,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Caller {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let token = req
            .headers()
            .get_one("Authorization")
            .and_then(|h| {
                h.strip_prefix("Bearer ")
                    .or_else(|| h.strip_prefix("bearer "))
            })
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Outcome::Success(Caller {
            token,
            cross_origin: is_cross_origin(req),
        })
    }
}

pub struct Reply {
    status: Status,
    body: Option<Value>,
    challenge: bool,
}

impl Reply {
    fn json(status: Status, body: Value) -> Self {
        Reply {
            status,
            body: Some(body),
            challenge: false,
        }
    }

    fn unauthorized(message: &str) -> Self {
        Reply {
            status: Status::Unauthorized,
            body: Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32001, "message": message } }),
            ),
            challenge: true,
        }
    }
}

impl<'r> Responder<'r, 'static> for Reply {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut res = Response::build();
        res.status(self.status)
            .header(Header::new("Cache-Control", "no-store"));
        if self.challenge {
            res.header(Header::new("WWW-Authenticate", "Bearer realm=\"nrzNews\""));
        }
        if let Some(body) = self.body {
            let text = body.to_string();
            res.header(ContentType::JSON)
                .sized_body(text.len(), Cursor::new(text));
        }
        res.ok()
    }
}

#[post("/mcp", data = "<body>")]
async fn post_mcp(caller: Caller, body: String, pool: &State<SqlitePool>) -> Reply {
    if caller.cross_origin {
        return Reply::json(
            Status::Forbidden,
            json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32000, "message": "Anfrage von einer fremden Seite abgewiesen." } }),
        );
    }
    let Some(token) = caller.token else {
        return Reply::unauthorized("API-Token fehlt (Authorization: Bearer nrznews_…, anzulegen unter Einstellungen → Konto).");
    };
    let user = match tokens::authenticate(pool, &token).await {
        Ok(Some(u)) => u,
        Ok(None) => return Reply::unauthorized("API-Token ungültig oder Konto gesperrt."),
        Err(e) => {
            tracing::error!("MCP-Token prüfen: {e}");
            return Reply::json(
                Status::InternalServerError,
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32603, "message": "Interner Fehler" } }),
            );
        }
    };
    let msg: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return Reply::json(
                Status::BadRequest,
                mcp::parse_error(format!("Kein gültiges JSON: {e}")),
            )
        }
    };
    match mcp::handle(pool, &user, msg).await {
        Some(answer) => Reply::json(Status::Ok, answer),
        None => Reply {
            status: Status::Accepted,
            body: None,
            challenge: false,
        },
    }
}

/// Kein SSE-Strom vom Server (erlaubt laut Spezifikation: 405).
#[get("/mcp")]
fn get_mcp() -> Reply {
    Reply {
        status: Status::MethodNotAllowed,
        body: None,
        challenge: false,
    }
}
