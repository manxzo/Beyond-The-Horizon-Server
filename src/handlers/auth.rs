use crate::models::all_models::UserRole;
use chrono::{Duration, Utc};
use dotenvy::dotenv;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::env;
use uuid::Uuid;
/// Structure representing JWT claims
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub id: Uuid, // User ID as string
    pub username: String,
    pub role: UserRole, // Role as string
    pub exp: usize,     // Expiration timestamp
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
pub fn validate_jwt(token: &str) -> Result<Claims, Box<dyn std::error::Error>> {
    dotenv().ok();

    // Improved error handling for JWT_SECRET retrieval
    let secret_key = env::var("JWT_SECRET").map_err(|e| -> Box<dyn std::error::Error> {
        format!("Failed to retrieve JWT_SECRET: {}", e).into()
    })?;

    // Validate the token with better error context
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret_key.as_ref()),
        &Validation::new(jsonwebtoken::Algorithm::HS256),
    )
    .map_err(|e| -> Box<dyn std::error::Error> {
        // Provide more context about the validation error
        match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                format!("Token has expired: {}", e).into()
            }
            jsonwebtoken::errors::ErrorKind::InvalidSignature => {
                format!("Invalid token signature: {}", e).into()
            }
            _ => format!("Token validation failed: {}", e).into(),
        }
    })?;

    Ok(token_data.claims)
}
