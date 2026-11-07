use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const ITEMS: [&str; 5] = ["Hand", "Sword", "Axe", "Fish", "Stick"];
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
	Range,
	Graph,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
	pub hit_delay_fix: bool,
	pub auto_clicker: bool,
	pub mode: Mode,
	pub min_cps: u8,
	pub max_cps: u8,
	pub graph: [u8; 11],
	pub block_check: bool,
	pub items: [bool; 5],
	pub right_key: u16,
	pub activate_with_key: bool,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			hit_delay_fix: true,
			auto_clicker: true,
			mode: Mode::Graph,
			min_cps: 10,
			max_cps: 20,
			graph: [3, 4, 5, 8, 15, 30, 15, 8, 5, 4, 3],
			block_check: true,
			items: [true; 5],
			right_key: b'R' as u16,
			activate_with_key: false,
		}
	}
}

impl Config {
	pub fn sanitize(&mut self) {
		self.min_cps = self.min_cps.clamp(10, 20);
		self.max_cps = self.max_cps.clamp(self.min_cps, 20);
		if !matches!(self.right_key, 0x30..=0x39 | 0x41..=0x5A | 0x70..=0x7B) {
			self.right_key = b'R' as u16;
		}
		if self.graph.iter().map(|&v| v as u32).sum::<u32>() != 100 {
			let original = self.graph;
			self.graph = normalize(&original.map(u32::from), 100);
		}
	}

	pub fn set_probability(&mut self, index: usize, value: u8) {
		if index >= self.graph.len() { return; }
		let value = value.min(100);
		let mut weights = self.graph.map(u32::from);
		let remaining = 100 - value as u32;
		weights[index] = 0;
		let mut adjusted = if weights.iter().all(|&weight| weight == 0) {
			let mut equal = [1u32; 11];
			equal[index] = 0;
			normalize(&equal, remaining)
		} else {
			normalize(&weights, remaining)
		};
		adjusted[index] = value;
		self.graph = adjusted;
	}
}

fn normalize(weights: &[u32; 11], total: u32) -> [u8; 11] {
	let sum = weights.iter().sum::<u32>();
	let mut output = [0u8; 11];
	if sum == 0 {
		for index in 0..total as usize {
			output[index % 11] += 1;
		}
		return output;
	}
	let mut remainder = Vec::with_capacity(11);
	let mut assigned = 0;
	for (i, &weight) in weights.iter().enumerate() {
		let product = weight * total;
		output[i] = (product / sum) as u8;
		assigned += output[i] as u32;
		remainder.push((product % sum, i));
	}
	remainder.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
	for &(_, index) in remainder.iter().take((total - assigned) as usize) {
		output[index] += 1;
	}
	output
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
	pub pid: u32,
	pub ready: bool,
	pub mapped: bool,
	pub in_world: bool,
	pub gui_open: bool,
	pub targeting_block: bool,
	pub target_known: bool,
	pub held_known: bool,
	pub held_item: String,
	pub error: String,
}

pub fn data_dir() -> PathBuf {
	std::env::var_os("APPDATA")
		.map(PathBuf::from)
		.unwrap_or_else(std::env::temp_dir)
		.join("LunarInjector")
}

pub fn config_path() -> PathBuf {
	data_dir().join("settings.json")
}

pub fn status_path(pid: u32) -> PathBuf {
	data_dir().join(format!("status-{pid}.json"))
}

pub fn load_config() -> Config {
	let mut config = std::fs::read(config_path())
		.ok()
		.and_then(|data| serde_json::from_slice::<Config>(&data).ok())
		.unwrap_or_default();
	config.sanitize();
	config
}

pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
	let parent = path.parent().ok_or_else(|| std::io::Error::other("Missing output directory"))?;
	std::fs::create_dir_all(parent)?;
	let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
	let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), sequence));
	std::fs::write(&tmp, data)?;
	let result = std::fs::rename(&tmp, path);
	if result.is_err() { let _ = std::fs::remove_file(tmp); }
	result
}

pub fn save_config(config: &Config) -> std::io::Result<()> {
	let json = serde_json::to_vec_pretty(config).map_err(std::io::Error::other)?;
	write_atomic(&config_path(), &json)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_sum_to_100() {
		assert_eq!(Config::default().graph.iter().map(|&v| v as u32).sum::<u32>(), 100);
	}

	#[test]
	fn editing_any_probability_preserves_total_and_value() {
		for index in 0..11 {
			for value in 0..=100 {
				let mut config = Config::default();
				config.set_probability(index, value);
				assert_eq!(config.graph[index], value);
				assert_eq!(config.graph.iter().map(|&v| v as u32).sum::<u32>(), 100);
			}
		}
	}

	#[test]
	fn moving_all_probability_to_another_point() {
		let mut config = Config::default();
		config.set_probability(0, 100);
		config.set_probability(0, 0);
		assert_eq!(config.graph[0], 0);
		assert_eq!(config.graph.iter().map(|&v| v as u32).sum::<u32>(), 100);
	}

	#[test]
	fn invalid_probability_index_leaves_distribution_unchanged() {
		let mut config = Config::default();
		let before = config.graph;
		config.set_probability(11, 100);
		assert_eq!(config.graph, before);
	}

	#[test]
	fn loading_older_status_defaults_to_safe_unknown_values() {
		let status: Status = serde_json::from_str(r#"{"pid":10,"ready":true,"mapped":true}"#).unwrap();
		assert!(!status.target_known);
		assert!(!status.held_known);
	}

	#[test]
	fn atomic_write_replaces_existing_bytes() {
		let path = std::env::temp_dir().join(format!("lunar-injector-test-{}", std::process::id()));
		write_atomic(&path, b"first").unwrap();
		write_atomic(&path, b"second").unwrap();
		assert_eq!(std::fs::read(&path).unwrap(), b"second");
		std::fs::remove_file(path).unwrap();
	}

	#[test]
	fn sanitize_clamps_range_and_repairs_graph() {
		let mut config = Config::default();
		config.min_cps = 40;
		config.max_cps = 0;
		config.graph = [0; 11];
		config.right_key = 1;
		config.sanitize();
		assert_eq!((config.min_cps, config.max_cps), (20, 20));
		assert_eq!(config.right_key, b'R' as u16);
		assert_eq!(config.graph.iter().map(|&v| v as u32).sum::<u32>(), 100);
	}
}
