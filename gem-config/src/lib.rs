pub mod config;
pub mod t3;

pub use config::Config;

#[cfg(test)]
mod tests {
    #[test]
    fn basic() {
        let data = include_bytes!("../../config.json");
        serde_json::from_slice::<super::Config>(data).unwrap();
    }
}
