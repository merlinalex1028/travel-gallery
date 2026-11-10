use crate::{catalog::Catalog, platform_power};
use std::time::{Duration, Instant};
pub(crate) struct PowerCache {
    pub value: Option<bool>,
    pub checked: Instant,
}
impl Default for PowerCache {
    fn default() -> Self {
        Self {
            value: None,
            checked: Instant::now() - Duration::from_secs(10),
        }
    }
}
impl Catalog {
    pub fn on_battery(&self) -> Option<bool> {
        let mut power = self.inner.power.lock().unwrap();
        if power.checked.elapsed() >= Duration::from_secs(5) {
            power.value = platform_power::on_battery();
            power.checked = Instant::now();
        }
        power.value
    }
}
