//! 地理位置和距离计算工具
//!
//! 提供 Haversine 公式计算地球表面两点间的大圆距离

use std::f64::consts::PI;

/// 地球半径（千米）
const EARTH_RADIUS_KM: f64 = 6371.0;

/// 使用 Haversine 公式计算两个地理坐标之间的距离
///
/// # Arguments
/// * `lat1` - 点 1 纬度（度数）
/// * `lon1` - 点 1 经度（度数）
/// * `lat2` - 点 2 纬度（度数）
/// * `lon2` - 点 2 经度（度数）
///
/// # Returns
/// 两点之间的大圆距离（千米）
///
/// # Example
/// ```
/// use signaling::geo::haversine_distance;
///
/// // 北京到上海的距离
/// let beijing = (39.9042, 116.4074);
/// let shanghai = (31.2304, 121.4737);
/// let distance = haversine_distance(beijing.0, beijing.1, shanghai.0, shanghai.1);
/// assert!((distance - 1067.0).abs() < 10.0); // 约 1067 公里
/// ```
pub fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    // 转换为弧度
    let lat1_rad = lat1 * PI / 180.0;
    let lat2_rad = lat2 * PI / 180.0;
    let delta_lat = (lat2 - lat1) * PI / 180.0;
    let delta_lon = (lon2 - lon1) * PI / 180.0;

    // Haversine 公式
    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    EARTH_RADIUS_KM * c
}

/// 地理坐标点
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoPoint {
    /// 创建新的地理坐标点
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    /// 计算到另一个点的距离（千米）
    pub fn distance_to(&self, other: &GeoPoint) -> f64 {
        haversine_distance(
            self.latitude,
            self.longitude,
            other.latitude,
            other.longitude,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_haversine_same_point() {
        let distance = haversine_distance(39.9042, 116.4074, 39.9042, 116.4074);
        assert!(distance.abs() < 0.001);
    }

    #[test]
    fn test_haversine_beijing_shanghai() {
        // 北京: 39.9042°N, 116.4074°E
        // 上海: 31.2304°N, 121.4737°E
        // 实际距离约 1067 公里
        let distance = haversine_distance(39.9042, 116.4074, 31.2304, 121.4737);
        assert!((distance - 1067.0).abs() < 20.0); // 允许 20km 误差
    }

    #[test]
    fn test_haversine_new_york_london() {
        // 纽约: 40.7128°N, 74.0060°W
        // 伦敦: 51.5074°N, 0.1278°W
        // 实际距离约 5570 公里
        let distance = haversine_distance(40.7128, -74.0060, 51.5074, -0.1278);
        assert!((distance - 5570.0).abs() < 50.0); // 允许 50km 误差
    }

    #[test]
    fn test_geopoint_distance() {
        let beijing = GeoPoint::new(39.9042, 116.4074);
        let shanghai = GeoPoint::new(31.2304, 121.4737);
        let distance = beijing.distance_to(&shanghai);
        assert!((distance - 1067.0).abs() < 20.0);
    }

    #[test]
    fn test_geopoint_equality() {
        let p1 = GeoPoint::new(39.9042, 116.4074);
        let p2 = GeoPoint::new(39.9042, 116.4074);
        assert_eq!(p1, p2);
    }
}
