use actix_web::middleware::Logger;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder, Result};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;
use structopt::StructOpt;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

#[derive(Debug, StructOpt)]
#[structopt(
    name = "director",
    about = "Custom IT automation workflows, no kubernetes inside!"
)]
struct Opt {
    /// Where to find built front files ; default is to look to front/build (it is used
    /// to debug release builds)
    #[structopt(
        long = "document-root",
        parse(from_os_str),
        default_value = "front/build"
    )]
    document_root: PathBuf,
}

async fn greet(req: HttpRequest) -> impl Responder {
    let name = req.match_info().get("name").unwrap_or("World");
    format!("Hello {}!", &name)
}

#[derive(Serialize)]
struct Version {
    director: String,
    core: String,
    protocol: String,
    query_parser: String,
}

async fn version() -> Result<HttpResponse> {
    //tokio::time::delay_for(Duration::from_secs(2)).await;
    Ok(HttpResponse::Ok().json(Version {
        director: VERSION.into(),
        core: funtonic::VERSION.into(),
        protocol: funtonic::PROTOCOL_VERSION.into(),
        query_parser: funtonic::QUERY_PARSER_VERSION.into(),
    }))
}

const LOG4RS_CONFIG: &'static str = "/etc/funtonic/director-log4rs.yaml";

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    log4rs_gelf::init_file(LOG4RS_CONFIG, None).unwrap_or_else(|e| {
        eprintln!("Cannot initialize logger from {} - {}", LOG4RS_CONFIG, e);
        eprintln!("Trying with dev assets!");
        log4rs_gelf::init_file("executor/assets/log4rs.yaml", None)
            .expect("Cannot open executor/assets/log4rs.yaml");
    });
    HttpServer::new(|| {
        App::new()
            .wrap(Logger::default())
            .route(
                "/",
                web::get().to(|| {
                    HttpResponse::Found()
                        .header(http::header::LOCATION, "/index.html")
                        .finish()
                        .into_body()
                }),
            )
            .route("/api/version", web::to(version))
            .default_service(
                actix_files::Files::new("./", "director/front/build")
                    .use_etag(true)
                    .use_last_modified(true),
            )
    })
    .bind("127.0.0.1:8000")?
    .run()
    .await
}
