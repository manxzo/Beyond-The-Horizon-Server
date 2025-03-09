use sqlx::PgPool;

pub async fn check_db_connection(pool: &PgPool) -> bool {
    match pool.acquire().await {
        Ok(_) => {
           
            true
        }
        Err(e) => {
            
            log::error!("Database connection check failed: {}", e);
            false
        }
    }
}
