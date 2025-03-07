use chrono::{Duration, Utc};
use dotenvy::dotenv;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::env;
use uuid::Uuid;
use crate::models::all_models::UserRole;
/// Structure representing JWT claims
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub id: Uuid, // User ID as string
    pub username: String,
    pub role: UserRole, // Role as string
    pub exp: usize,   // Expiration timestamp
}

/// Generates a JWT token for a given user
pub fn generate_jwt(
    user_id: Uuid,
    username: String,
    role: UserRole,
) -> Result<String, jsonwebtoken::errors::Error> {
    dotenv().ok();
    let secret_key = env::var("JWT_SECRET").expect("JWT_SECRET must be set");

    let expiration = Utc::now() + Duration::hours(8);
    let claims = Claims {
        id: user_id,
        username: username.to_string(),
        role: role,
        exp: expiration.timestamp() as usize,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret_key.as_ref()),
    )
}

/// Validates a JWT token and extracts the user information
pub fn validate_jwt(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    dotenv().ok();
    let secret_key = env::var("JWT_SECRET").expect("JWT_SECRET must be set");

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret_key.as_ref()),
        &Validation::new(jsonwebtoken::Algorithm::HS256),
    )?;

    Ok(token_data.claims)
}

/// Helper function to get the user ID as a string from Claims
pub fn get_id_str(claims: &Claims) -> String {
    claims.id.to_string()
}

/// Helper function to get the role as a string from Claims
pub fn get_role_str(claims: &Claims) -> String {
    claims.role.to_string()
}
