use sqlx::PgPool;
use dotenvy::dotenv;
use std::env;

pub async fn connect_db() -> PgPool {
    dotenv().ok();
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    PgPool::connect(&database_url).await.expect("Failed to connect to database")
}
