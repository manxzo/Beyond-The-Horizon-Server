use crate::models::all_models::UserRole;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Structure representing user identity claims
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub id: Uuid, // User ID
    pub username: String,
    pub role: UserRole, // User role
    pub exp: usize,     // Expiration timestamp
}
