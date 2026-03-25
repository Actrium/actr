# actr-framework Utilities

This module provides optional utility functions, independent of the core framework interfaces.

## GeoIP Geolocation Lookup

Provides IP address to geographic coordinate conversion based on MaxMind GeoLite2 database.

### Quick Start

#### 1. Enable geoip feature

Add to `Cargo.toml`:

```toml
[dependencies]
actr-framework = { path = "../actr/crates/framework", features = ["geoip"] }
```

#### 2. Prepare GeoIP Database

**Automatic Download (Recommended):**

Automatically downloads on first call to `GeoIpService::new()` (requires environment variable):

```bash
# Get License Key: https://www.maxmind.com/en/geolite2/signup
export MAXMIND_LICENSE_KEY="your-key-here"

# Automatically downloads (~70MB, approx 30 seconds) on first run
cargo run --features geoip
```

**Manual Download (Production):**

```bash
curl -o GeoLite2-City.tar.gz \
  "https://download.maxmind.com/app/geoip_download?edition_id=GeoLite2-City&license_key=YOUR_KEY&suffix=tar.gz"
tar -xzf GeoLite2-City.tar.gz --strip-components=1 -C data/geoip/ "*/GeoLite2-City.mmdb"
```

#### 3. Use in Actor

```rust
use actr_framework::util::geoip::GeoIpService;
use actr_protocol::{RegisterRequest, ServiceLocation};

// Initialize GeoIP service
let geoip = GeoIpService::new("data/geoip/GeoLite2-City.mmdb")?;

// Lookup coordinates for local IP
let my_ip = local_ip_address::local_ip()?;
let location = geoip.lookup(my_ip).map(|(lat, lon)| ServiceLocation {
    region: "auto-detected".to_string(),
    latitude: Some(lat),
    longitude: Some(lon),
});

// Provide coordinates during registration
let request = RegisterRequest {
    realm: Realm { realm_id: 1 },
    actr_type: my_type,
    geo_location: location,
    // ... other fields
};
```

### API Documentation

#### `GeoIpService::new(db_path)`

Initialize GeoIP service.

**Arguments:**
- `db_path` - Path to GeoLite2-City.mmdb database file

**Returns:**
- `Result<GeoIpService>` - Service instance on success, error on failure

**Errors:**
- Database file does not exist
- Database format error

#### `GeoIpService::lookup(ip)`

Lookup geographic coordinates for an IP address.

**Arguments:**
- `ip: IpAddr` - IP address to lookup

**Returns:**
- `Option<(f64, f64)>` - `(latitude, longitude)` on success, `None` on failure

**Notes:**
- Accuracy is city-level (50-100km error)
- Intranet IPs usually cannot be looked up
- Some public IPs may not be in the database

### Full Example

```rust
use actr_framework::util::geoip::GeoIpService;
use actr_framework::{Workload, Context};
use actr_protocol::*;
use anyhow::Result;

pub struct MyService {
    geoip: GeoIpService,
}

impl MyService {
    pub fn new() -> Result<Self> {
        let geoip = GeoIpService::new("data/geoip/GeoLite2-City.mmdb")?;
        Ok(Self { geoip })
    }

    fn get_my_location(&self) -> Option<ServiceLocation> {
        // Get local IP
        let my_ip = local_ip_address::local_ip().ok()?;

        // Lookup coordinates
        self.geoip.lookup(my_ip).map(|(lat, lon)| ServiceLocation {
            region: format!("auto-{}", my_ip),
            latitude: Some(lat),
            longitude: Some(lon),
        })
    }
}

#[async_trait]
impl Workload for MyService {
    type Dispatcher = MyServiceDispatcher;

    async fn on_start<C: Context>(&mut self, ctx: &C) -> ActorResult<()> {
        // Get coordinates
        let location = self.get_my_location();
        let actor_type = ctx.self_id().r#type;
        // Register to signaling service (with coordinates)
        let register_req = RegisterRequest {
            realm: Realm { realm_id: 1 },
            actr_type,
            geo_location: location,
            // ... other fields
        };

        // Send registration request
        // ctx.call(&signaling_dest, register_req).await?;

        Ok(())
    }
}
```

### Performance Characteristics

- **Database Size** - ~70MB (GeoLite2-City)
- **Lookup Latency** - In-memory lookup, < 1ms
- **Accuracy** - City-level, 50-100km error range
- **Coverage** - Most public IPs, intranet IPs cannot be looked up

### Troubleshooting

#### Database Load Failure

```
Error: Failed to open GeoIP database at "data/geoip/GeoLite2-City.mmdb"
```

**Solution:**
1. Confirm file exists: `ls -lh data/geoip/GeoLite2-City.mmdb`
2. Check file permissions: `chmod 644 data/geoip/GeoLite2-City.mmdb`
3. Set environment variable and rerun (auto download): `export MAXMIND_LICENSE_KEY="your-key" && cargo run --features geoip`
4. Or follow "Manual Download" steps above

#### IP Address Not Found

```
debug: GeoIP lookup: 192.168.1.1 not in database
```

**Explanation:**
- Intranet IPs (192.168.x.x, 10.x.x.x, 172.16-31.x.x) are not in the database
- Some public IPs may also not be in the database
- This is normal, returning `None` is expected

#### Inaccurate Coordinates

**Cause:** GeoLite2 accuracy is city-level (50-100km error)

**Solution:**
- Upgrade to GeoIP2 Precision (paid, accuracy up to 10km)
- Or provide accurate coordinates via configuration file
- Use GPS for mobile devices

### Feature Flag

GeoIP functionality is optional, controlled via feature flag:

```toml
# Default (without GeoIP)
actr-framework = { path = "..." }

# Enable GeoIP
actr-framework = { path = "...", features = ["geoip"] }
```

When `geoip` feature is disabled:
- `GeoIpService::new()` returns error
- `GeoIpService::lookup()` always returns `None`
- No dependency on `maxminddb` crate
- Reduces compilation time and binary size

### Best Practices

1. **Initialization Timing** - Initialize GeoIpService at Actor startup (not on every lookup)
2. **Error Handling** - Log database load failures but do not prevent Actor startup
3. **Fallback Strategy** - Use coordinates from configuration file on lookup failure
4. **Database Updates** - Update GeoLite2 database monthly (MaxMind updates weekly)

### Related Documentation

- MaxMind GeoLite2: https://dev.maxmind.com/geoip/geolite2-free-geolocation-data
- Signaling Service Geographic Load Balancing: `actrix-signaling/crates/signaling/README_GEOIP.md`
