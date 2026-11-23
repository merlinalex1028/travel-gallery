use crate::{error::Result, query};
use rusqlite::{params_from_iter, Connection};
use serde_json::{json, Value};
pub fn query(c: &Connection, request: &Value) -> Result<Value> {
    let filter: query::Filter =
        serde_json::from_value(request.get("filter").cloned().unwrap_or(json!({})))?;
    let (condition, parameters) = query::scoped_conditions(c, &filter)?;
    let condition=format!("{condition} AND a.gps_lat BETWEEN -90 AND 90 AND a.gps_lon BETWEEN -180 AND 180 AND a.capture_local IS NOT NULL AND a.capture_time_source IN ('exif','quicktime','manual')");
    let total: i64 = c.query_row(
        &format!("SELECT count(*){} WHERE {condition}", query::FROM),
        params_from_iter(parameters.iter()),
        |r| r.get(0),
    )?;
    let mut statement=c.prepare(&format!("SELECT a.capture_local,round(a.gps_lon,2),round(a.gps_lat,2),a.capture_tz_min {} WHERE {condition} ORDER BY a.capture_local,a.id LIMIT 5001",query::FROM))?;
    let rows = statement.query_map(params_from_iter(parameters.iter()), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, f64>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, Option<i64>>(3)?,
        ))
    })?;
    let mut features = vec![];
    let mut coordinates = vec![];
    let mut label = String::new();
    let mut previous: Option<(chrono::NaiveDateTime, Option<i64>, f64)> = None;
    let mut included = 0;
    let mut unknown = 0;
    let mut skipped = 0;
    let finish = |features: &mut Vec<Value>,
                  coordinates: &mut Vec<[f64; 2]>,
                  date: &str,
                  tz: Option<i64>| {
        if coordinates.len() > 1 {
            features.push(json!({"type":"Feature","properties":{"date":date,"unknownTimezone":tz.is_none()},"geometry":{"type":"LineString","coordinates":coordinates}}));
        }
        coordinates.clear();
    };
    for (index, row) in rows.enumerate() {
        if index == 5000 {
            break;
        }
        let (local, lon, lat, tz) = row?;
        let Ok(time) = chrono::NaiveDateTime::parse_from_str(&local, "%Y-%m-%d %H:%M:%S") else {
            skipped += 1;
            continue;
        };
        let date = time.date().to_string();
        if previous.is_some_and(|(before, before_tz, before_lon)| {
            date != label
                || time.signed_duration_since(before).num_hours() >= 6
                || before_tz != tz
                || (before_lon - lon).abs() > 180.
        }) {
            finish(&mut features, &mut coordinates, &label, previous.unwrap().1);
        }
        label = date;
        included += 1;
        if tz.is_none() {
            unknown += 1;
        }
        if coordinates.last() != Some(&[lon, lat]) {
            coordinates.push([lon, lat]);
        }
        previous = Some((time, tz, lon));
    }
    finish(
        &mut features,
        &mut coordinates,
        &label,
        previous.and_then(|p| p.1),
    );
    Ok(
        json!({"type":"FeatureCollection","features":features,"totalPhotos":total,"includedPhotos":included,"truncated":total>5000,"unknownTimezone":unknown,"skippedInvalidTimes":skipped}),
    )
}
