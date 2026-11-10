//! Cloud instance presets (step 8P, RULES 102 b): `instance: aws/m5.large` on a node fills its
//! `cpu.cores`, its network card (`bandwidth`) and the limit of its `memory`. One embedded table,
//! like the flavor presets ([`crate::presets`]); a value the user writes always wins.
//!
//! **Every number is an estimate**, taken from the providers' public instance pages as of
//! **2026-11** and rounded; refresh the table when a provider changes a family. Three traps, which
//! the docs repeat:
//! - a vCPU is a hardware thread, not a core, and the engine counts it as a full core, so a preset
//!   is an **upper bound** on compute capacity; per-core speed differs between families (a preset
//!   gives capacity, not speed: service times stay yours);
//! - the network figure is the *peak* ("up to") rate; sustained rates of the small sizes are
//!   lower and burst on credits, which are not modelled;
//! - burstable types (`t3`, `e2-medium`) run above their baseline only on CPU credits, which are
//!   not modelled: they are listed at their full vCPU count, an upper bound.

/// One instance type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instance {
    /// `provider/type`, lower case: what `instance:` takes.
    pub name: &'static str,
    /// vCPUs (hardware threads).
    pub vcpu: u16,
    /// Memory in MiB.
    pub memory_mib: u32,
    /// Peak network rate in megabits per second.
    pub net_mbps: u32,
}

impl Instance {
    /// The peak network rate in bytes per second.
    #[must_use]
    pub const fn net_bytes_per_s(&self) -> u64 {
        self.net_mbps as u64 * 1_000_000 / 8
    }

    /// The memory in bytes.
    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_mib as u64 * (1 << 20)
    }
}

const fn i(name: &'static str, vcpu: u16, mem_gib: u32, net_mbps: u32) -> Instance {
    Instance {
        name,
        vcpu,
        memory_mib: mem_gib * 1024,
        net_mbps,
    }
}

/// The table, sorted by name (looked up by binary search).
pub const INSTANCES: &[Instance] = &[
    i("aws/c6i.2xlarge", 8, 16, 12_500),
    i("aws/c6i.4xlarge", 16, 32, 12_500),
    i("aws/c6i.large", 2, 4, 12_500),
    i("aws/c6i.xlarge", 4, 8, 12_500),
    i("aws/m5.2xlarge", 8, 32, 10_000),
    i("aws/m5.4xlarge", 16, 64, 10_000),
    i("aws/m5.large", 2, 8, 10_000),
    i("aws/m5.xlarge", 4, 16, 10_000),
    i("aws/r6i.2xlarge", 8, 64, 12_500),
    i("aws/r6i.large", 2, 16, 12_500),
    i("aws/r6i.xlarge", 4, 32, 12_500),
    i("aws/t3.large", 2, 8, 5_000),
    i("aws/t3.medium", 2, 4, 5_000),
    i("aws/t3.micro", 2, 1, 5_000),
    i("aws/t3.small", 2, 2, 5_000),
    i("azure/standard_d2s_v5", 2, 8, 12_500),
    i("azure/standard_d4s_v5", 4, 16, 12_500),
    i("azure/standard_d8s_v5", 8, 32, 12_500),
    i("gcp/e2-medium", 2, 4, 2_000),
    i("gcp/e2-standard-2", 2, 8, 4_000),
    i("gcp/e2-standard-4", 4, 16, 8_000),
    i("gcp/n2-standard-2", 2, 8, 10_000),
    i("gcp/n2-standard-4", 4, 16, 10_000),
    i("gcp/n2-standard-8", 8, 32, 16_000),
];

/// The instance named `name` (case-insensitive), if the table has it.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static Instance> {
    let key = name.trim().to_ascii_lowercase();
    INSTANCES
        .binary_search_by(|x| x.name.cmp(key.as_str()))
        .ok()
        .map(|k| &INSTANCES[k])
}

/// A hint for an unknown name: the table's names that share its provider prefix, or the
/// providers when there is none.
#[must_use]
pub fn hint(name: &str) -> String {
    let key = name.trim().to_ascii_lowercase();
    let provider = key.split('/').next().unwrap_or_default();
    let prefix = format!("{provider}/");
    let same: Vec<&str> = INSTANCES
        .iter()
        .map(|x| x.name)
        .filter(|n| n.starts_with(&prefix))
        .collect();
    if same.is_empty() {
        "write provider/type, with the provider aws, gcp or azure, like aws/m5.large".into()
    } else {
        format!("known {provider} types: {}", same.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_unique_so_binary_search_finds_every_entry() {
        assert!(INSTANCES.windows(2).all(|w| w[0].name < w[1].name));
        for x in INSTANCES {
            assert_eq!(lookup(x.name), Some(x));
        }
    }

    #[test]
    fn lookup_ignores_case_and_space_and_refuses_the_unknown() {
        let m5 = lookup("  AWS/M5.Large ").unwrap();
        assert_eq!((m5.vcpu, m5.memory_mib), (2, 8192));
        assert_eq!(m5.net_bytes_per_s(), 1_250_000_000);
        assert!(lookup("aws/m5.huge").is_none());
        assert!(hint("aws/m5.huge").contains("aws/m5.large"));
        assert!(hint("mars/x1").contains("provider/type"));
    }
}

#[cfg(test)]
mod ui_table {
    use super::*;

    /// Studio keeps the same table (`ui/src/panels/details/instances.ts`) to fill its picker; a
    /// drift would offer a name the server refuses or hide one it knows.
    #[test]
    fn the_studio_picker_lists_exactly_the_servers_table() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../ui/src/panels/details/instances.ts"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            return; // a source package without the UI
        };
        let in_ui: Vec<&str> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("t(\""))
            .filter_map(|l| l.split('"').next())
            .collect();
        let in_rust: Vec<&str> = INSTANCES.iter().map(|x| x.name).collect();
        assert_eq!(in_ui, in_rust);
        // and the figures, not only the names: vCPU, GiB, Gbit/s
        for (line, x) in text
            .lines()
            .filter(|l| l.trim().starts_with("t(\""))
            .zip(INSTANCES)
        {
            let nums: Vec<f64> = line
                .trim()
                .trim_end_matches([')', ','])
                .split(", ")
                .skip(1)
                .map(|n| n.parse().unwrap())
                .collect();
            assert_eq!(nums[0], f64::from(x.vcpu), "{}", x.name);
            assert_eq!(nums[1], f64::from(x.memory_mib / 1024), "{}", x.name);
            assert_eq!(nums[2], f64::from(x.net_mbps) / 1000.0, "{}", x.name);
        }
    }
}
