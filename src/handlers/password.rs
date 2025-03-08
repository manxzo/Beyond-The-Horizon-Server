
use argon2::{Argon2, PasswordHasher, PasswordVerifier,password_hash};
use password_hash::{SaltString, PasswordHash, rand_core::OsRng};
/// Hash a password using Argon2
pub fn hash_password(password: &str) -> Result<String, password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    
    let password_hash = argon2.hash_password(password.as_bytes(), &salt)?;
    Ok(password_hash.to_string())
}

/// Verify a password
pub fn verify_password(password: &str, hash: &str) -> Result<bool, password_hash::Error> {
    let parsed_hash = PasswordHash::new(hash)?;
    let argon2 = Argon2::default();
    
    Ok(argon2.verify_password(password.as_bytes(), &parsed_hash).is_ok())
}
