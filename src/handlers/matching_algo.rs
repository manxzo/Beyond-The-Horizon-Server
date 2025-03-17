use crate::models::all_models::MatchUser;
use chrono::{Datelike, Utc};
use geoutils::Location;

/// Calculate match score between two users
pub fn calculate_match_score(member: &MatchUser, sponsor: &MatchUser) -> f32 {
    let mut score: f32 = 0.0;

    let member_age = Utc::now().year() - member.dob.year();
    let sponsor_age = Utc::now().year() - sponsor.dob.year();
    let age_diff = (member_age as i32 - sponsor_age as i32).abs();
    let age_score = 30.0 * (1.0 - (age_diff as f32 / 15.0)).max(0.0);
    score += age_score;

    if let (Some(member_loc), Some(sponsor_loc)) =
        (member.parse_location(), sponsor.parse_location())
    {
        let member_point = Location::new(member_loc.longitude, member_loc.latitude);
        let sponsor_point = Location::new(sponsor_loc.longitude, sponsor_loc.latitude);

        let distance = member_point.haversine_distance_to(&sponsor_point).meters();

        let location_score = if distance <= 10000.0 {
            30.0 // Same city or close distance
        } else if distance <= 50000.0 {
            20.0 // Nearby cities
        } else if distance <= 200000.0 {
            10.0 // Regional match
        } else {
            5.0 // Very distant
        };
        score += location_score;
    }

    if let (Some(member_interests), Some(sponsor_interests)) =
        (&member.interests, &sponsor.interests)
    {
        let common = member_interests
            .iter()
            .filter(|i| sponsor_interests.contains(i))
            .count();
        let interest_score = 20.0 * (common as f32 / member_interests.len().max(1) as f32);
        score += interest_score;
    }

    if let (Some(member_exp), Some(sponsor_exp)) = (&member.experience, &sponsor.experience) {
        let common = member_exp
            .iter()
            .filter(|e| sponsor_exp.contains(e))
            .count();
        let exp_score = 15.0 * (common as f32 / member_exp.len().max(1) as f32);
        score += exp_score;
    }

    if let (Some(member_avail), Some(sponsor_avail)) =
        (&member.available_days, &sponsor.available_days)
    {
        let common = member_avail
            .iter()
            .filter(|d| sponsor_avail.contains(d))
            .count();
        let avail_score = 10.0 * (common as f32 / member_avail.len().max(1) as f32);
        score += avail_score;
    }

    if let (Some(member_lang), Some(sponsor_lang)) = (&member.languages, &sponsor.languages) {
        let common = member_lang
            .iter()
            .filter(|l| sponsor_lang.contains(l))
            .count();
        let lang_score = 5.0 * (common as f32 / member_lang.len().max(1) as f32);
        score += lang_score;
    }

    let normalized_score = (score / 100.0) * 100.0;
    normalized_score.min(100.0)
}
