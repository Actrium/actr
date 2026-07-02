use super::*;

fn quota(cpu: f64, mem: u64) -> ResourceQuota {
    ResourceQuota {
        cpu_cores: cpu,
        memory_bytes: mem,
        network_bandwidth_bps: 100,
        disk_io_bps: 100,
    }
}

#[test]
fn quota_default_values() {
    let q = ResourceQuota::default();
    assert_eq!(q.cpu_cores, 1.0);
    assert_eq!(q.memory_bytes, 1024 * 1024 * 1024);
    assert_eq!(q.network_bandwidth_bps, 100 * 1024 * 1024);
    assert_eq!(q.disk_io_bps, 100 * 1024 * 1024);
}

#[test]
fn config_default_values() {
    let c = ResourceConfig::default();
    assert!(c.enable_limits);
    assert_eq!(c.monitoring_interval_seconds, 5);
    assert_eq!(c.warning_threshold, 0.8);
    assert_eq!(c.limit_threshold, 0.95);
}

#[test]
fn resource_usage_default_is_zero() {
    let u = ResourceUsage::default();
    assert_eq!(u.cpu_usage, 0.0);
    assert_eq!(u.memory_used_bytes, 0);
    assert_eq!(u.network_usage_bps, 0);
    assert_eq!(u.disk_io_bps, 0);
}

#[test]
fn disabled_limits_always_available() {
    let cfg = ResourceConfig {
        enable_limits: false,
        ..ResourceConfig::default()
    };
    let rm = ResourceManager::new(cfg, ResourceQuota::default());

    // Even an absurd request is "available" when limits are off.
    let huge = ResourceUsage {
        cpu_usage: 1000.0,
        memory_used_bytes: u64::MAX,
        network_usage_bps: 0,
        disk_io_bps: 0,
    };
    assert!(rm.check_resource_availability(&huge).unwrap());
}

#[test]
fn availability_ok_within_quota() {
    let rm = ResourceManager::new(ResourceConfig::default(), quota(4.0, 1024));
    let req = ResourceUsage {
        cpu_usage: 0.5, // 0.5 * 4 = 2 cores, available = 4 - 0 = 4
        memory_used_bytes: 512,
        network_usage_bps: 0,
        disk_io_bps: 0,
    };
    assert!(rm.check_resource_availability(&req).unwrap());
}

#[test]
fn availability_rejected_on_cpu_exhaustion() {
    let rm = ResourceManager::new(ResourceConfig::default(), quota(1.0, 1024));
    let req = ResourceUsage {
        cpu_usage: 0.5, // available cpu = 1 - 0 = 1; required = 0.5*1 = 0.5 <= 1 ok here
        memory_used_bytes: 0,
        network_usage_bps: 0,
        disk_io_bps: 0,
    };
    // Sanity: passes with room to spare.
    assert!(rm.check_resource_availability(&req).unwrap());

    // Now pre-allocate to consume most CPU, then re-check should fail.
    let mut rm = rm;
    rm.allocate_resources(&ResourceUsage {
        cpu_usage: 0.9,
        memory_used_bytes: 0,
        network_usage_bps: 0,
        disk_io_bps: 0,
    })
    .unwrap();
    // available cpu = 1 - 0.9*1 = 0.1; required 0.5*1 = 0.5 > 0.1 → reject
    assert!(!rm.check_resource_availability(&req).unwrap());
}

#[test]
fn availability_rejected_on_memory_exhaustion() {
    let rm = ResourceManager::new(ResourceConfig::default(), quota(1.0, 1000));
    let req = ResourceUsage {
        cpu_usage: 0.0,
        memory_used_bytes: 1500, // > 1000 available
        network_usage_bps: 0,
        disk_io_bps: 0,
    };
    assert!(!rm.check_resource_availability(&req).unwrap());
}

#[test]
fn allocate_updates_usage_and_getters() {
    let mut rm = ResourceManager::new(ResourceConfig::default(), quota(2.0, 1024));
    let req = ResourceUsage {
        cpu_usage: 0.25,
        memory_used_bytes: 300,
        network_usage_bps: 10,
        disk_io_bps: 20,
    };
    rm.allocate_resources(&req).unwrap();

    let usage = rm.get_usage();
    assert_eq!(usage.cpu_usage, 0.25);
    assert_eq!(usage.memory_used_bytes, 300);
    assert_eq!(usage.network_usage_bps, 10);
    assert_eq!(usage.disk_io_bps, 20);

    let q = rm.get_quota();
    assert_eq!(q.cpu_cores, 2.0);
    assert_eq!(q.memory_bytes, 1024);
}

#[test]
fn allocate_rejects_when_insufficient() {
    let mut rm = ResourceManager::new(ResourceConfig::default(), quota(1.0, 100));
    let req = ResourceUsage {
        cpu_usage: 0.0,
        memory_used_bytes: 200, // exceeds 100
        network_usage_bps: 0,
        disk_io_bps: 0,
    };
    let err = rm.allocate_resources(&req).unwrap_err();
    assert!(matches!(err, ActrError::Unavailable(_)));
    // Usage must remain unchanged on rejection.
    assert_eq!(rm.get_usage().memory_used_bytes, 0);
}

#[test]
fn release_saturates_and_does_not_underflow() {
    let mut rm = ResourceManager::new(ResourceConfig::default(), quota(1.0, 1024));
    // Allocate a little, then release more than allocated — must saturate, not panic.
    rm.allocate_resources(&ResourceUsage {
        cpu_usage: 0.1,
        memory_used_bytes: 50,
        network_usage_bps: 5,
        disk_io_bps: 5,
    })
    .unwrap();
    rm.release_resources(&ResourceUsage {
        cpu_usage: 0.9,
        memory_used_bytes: 9999,
        network_usage_bps: 9999,
        disk_io_bps: 9999,
    })
    .unwrap();

    let u = rm.get_usage();
    assert_eq!(u.cpu_usage, 0.0);
    assert_eq!(u.memory_used_bytes, 0);
    assert_eq!(u.network_usage_bps, 0);
    assert_eq!(u.disk_io_bps, 0);
}

#[test]
fn calculate_usage_ratio_reflects_allocation() {
    let mut rm = ResourceManager::new(
        ResourceConfig::default(),
        ResourceQuota {
            cpu_cores: 2.0,
            memory_bytes: 1000,
            network_bandwidth_bps: 200,
            disk_io_bps: 400,
        },
    );
    rm.allocate_resources(&ResourceUsage {
        cpu_usage: 0.5,         // cpu_ratio tracks raw usage fraction = 0.5
        memory_used_bytes: 250, // 250/1000 = 0.25
        network_usage_bps: 50,  // 50/200 = 0.25
        disk_io_bps: 100,       // 100/400 = 0.25
    })
    .unwrap();

    let r = rm.calculate_usage_ratio();
    assert_eq!(r.cpu_ratio, 0.5);
    assert!((r.memory_ratio - 0.25).abs() < f64::EPSILON);
    assert!((r.network_ratio - 0.25).abs() < f64::EPSILON);
    assert!((r.disk_ratio - 0.25).abs() < f64::EPSILON);
}

#[test]
fn ratio_zero_when_idle() {
    let rm = ResourceManager::new(ResourceConfig::default(), ResourceQuota::default());
    let r = rm.calculate_usage_ratio();
    assert_eq!(r.cpu_ratio, 0.0);
    assert_eq!(r.memory_ratio, 0.0);
    assert_eq!(r.network_ratio, 0.0);
    assert_eq!(r.disk_ratio, 0.0);
}
