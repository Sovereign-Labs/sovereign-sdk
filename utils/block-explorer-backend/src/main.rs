use axum::{routing::get, Router};
use clap::Parser;
use sqlx::PgPool;

fn api_v0_router(db: PgPool) -> Router {
    Router::new()
        .nest(
            "/api/v0",
            Router::new().route("/latest", get(routes::get_latest_block)),
        )
        .with_state(db)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    let db = PgPool::connect(&config.db_connection_url).await.unwrap();
    run_migrations(db.clone()).await?;
    let app = Router::new().nest("/api/v0", api_v0_router(db));

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn run_migrations(pool: sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(())
}

#[derive(Debug, Parser)]
struct Config {
    #[clap(short, long)]
    db_connection_url: String,
}

pub mod routes {
    use super::*;
    use axum::{extract::State, Json};
    use indoc::indoc;
    use serde_json::{json, Value};

    type AppState = State<PgPool>;

    pub async fn get_latest_block(State(db): AppState) -> Json<Value> {
        let row: (Vec<u8>, i64) = sqlx::query_as(indoc!(
            r#"
            SELECT (hash, height) FROM blocks
            ORDER BY height DESC
            LIMIT 1
            "#
        ))
        .fetch_one(&db)
        .await
        .unwrap();

        Json(json!({
            "hash": row.0,
            "height": row.1,
        }))
    }
}
