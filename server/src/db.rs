use anyhow::Context;
use sqlx::{Pool, Sqlite, sqlite::SqlitePoolOptions};

#[derive(Clone)]
pub struct AppState {
    pub db: Pool<Sqlite>,
    pub jwt_secret: String,
}

pub async fn init_db() -> anyhow::Result<Pool<Sqlite>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite::memory:")
        .await
        .context("Failed to connect to sqlite database")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run migrations")?;

    Ok(pool)
}
