use super::{process, Action, BeginRoute, Config, VisionConfig};
use crate::device_status::response;
use axum::{extract::rejection::JsonRejection, http::HeaderMap, Extension, Json};
use serde_json::{json, Value};
use sqlx::{MySqlPool, Row};
use std::sync::Arc;

fn parse(mut value: Value, action: Action) -> Option<(BeginRoute, Vec<i64>)> {
    let ids: Vec<i64> = value
        .get("route_ids")?
        .as_str()?
        .split(',')
        .map(|id| id.trim().parse::<i64>())
        .collect::<Result<_, _>>()
        .ok()?;
    if ids.is_empty() || ids.iter().any(|id| !(1..=u32::MAX as i64).contains(id)) {
        return None;
    }
    let object = value.as_object_mut()?;
    object.insert("route_id".into(), ids[0].into());
    object.insert("stop_id".into(), 1.into()); // Resolved independently for each route below.
    object.insert("traffic_type_id".into(), action.traffic_type().into());
    if action == Action::BeginMerged {
        object.insert("traffic_situation_id".into(), 1.into());
    }
    for field in ["trace_id", "bus_info"] {
        if let Some(Value::Number(number)) = object.get(field) {
            object.insert(field.into(), number.to_string().into());
        }
    }
    let request: BeginRoute = serde_json::from_value(value).ok()?;
    (request.valid()
        && (action == Action::BeginMerged || request.traffic_situation_id.is_some_and(|id| id > 0)))
    .then_some((request, ids))
}

#[derive(Clone, Copy)]
struct Stop {
    id: i64,
    latitude: f64,
    longitude: f64,
    radius: f64,
}

fn closest(stops: &[Stop], latitude: f64, longitude: f64) -> Option<i64> {
    let mut overall = None;
    let mut within = None;
    let mut overall_distance = f64::INFINITY;
    let mut within_distance = f64::INFINITY;
    for stop in stops {
        let lat_delta = (stop.latitude - latitude).to_radians();
        let lon_delta = (stop.longitude - longitude).to_radians();
        let a = (lat_delta / 2.0).sin().powi(2)
            + latitude.to_radians().cos()
                * stop.latitude.to_radians().cos()
                * (lon_delta / 2.0).sin().powi(2);
        let distance = 6_371_000.0 * 2.0 * a.clamp(0.0, 1.0).sqrt().asin();
        if distance < overall_distance {
            overall_distance = distance;
            overall = Some(stop.id);
        }
        if distance <= stop.radius && distance < within_distance {
            within_distance = distance;
            within = Some(stop.id);
        }
    }
    within.or(overall)
}

async fn select_stop(
    pool: &MySqlPool,
    route_id: i64,
    latitude: f64,
    longitude: f64,
) -> Result<i64, super::Error> {
    let rows = sqlx::query("SELECT CAST(s.id_stop AS SIGNED) AS id_stop, CAST(s.stop_latitude AS DOUBLE) AS latitude, CAST(s.stop_longitude AS DOUBLE) AS longitude, CAST(COALESCE(rsr.stop_ratio,0) AS DOUBLE) AS radius FROM routes r INNER JOIN routes_stops_relationship rsr ON r.id_route=rsr.id_route INNER JOIN stops s ON s.id_stop=rsr.id_stop WHERE r.id_route=? ORDER BY rsr.order_stop")
        .bind(route_id).fetch_all(pool).await?;
    let stops = rows
        .into_iter()
        .map(|row| {
            Ok(Stop {
                id: row.try_get("id_stop")?,
                latitude: row.try_get("latitude")?,
                longitude: row.try_get("longitude")?,
                radius: row.try_get("radius")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    closest(&stops, latitude, longitude)
        .ok_or_else(|| format!("No usable stops for route {route_id}").into())
}

#[tracing::instrument(skip_all)]
pub async fn begin_merged_route(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::BeginMerged).await
}

#[tracing::instrument(skip_all)]
pub async fn end_merged_route(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::EndMerged).await
}

async fn handle(
    pool: MySqlPool,
    config: Arc<Config>,
    vision: Arc<VisionConfig>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
    action: Action,
) -> Json<Value> {
    if !config.authorized(&headers) {
        return response(3, "Invalid Authentication");
    }
    let Ok(Json(body)) = body else {
        return response(3, "Invalid Json Object");
    };
    let Some(parameters) = body.get("ReaxiumParameters") else {
        return response(3, "Invalid Json Header");
    };
    let Some((template, ids)) = parameters
        .get("DeviceTraffic")
        .cloned()
        .and_then(|value| parse(value, action))
    else {
        return response(
            3,
            "Invalid Parameters received, please checkout the api documentation",
        );
    };
    let mut results = Vec::new();
    let mut failed = false;
    for route_id in ids {
        let mut request = template.clone();
        request.route_id = route_id;
        request.stop_id = match select_stop(&pool, route_id, request.latitude, request.longitude)
            .await
        {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(route_id, %error, "merged route stop selection failed");
                results.push(json!({"route_id":route_id,"code":3,"message":"Could not resolve a stop for this route; processing stopped."}));
                failed = true;
                break;
            }
        };
        let result = process(
            pool.clone(),
            config.clone(),
            vision.clone(),
            request.clone(),
            action,
        )
        .await;
        let success = result.0["ReaxiumResponse"]["code"] == 0;
        let saved = result.0["ReaxiumResponse"]["object"]["database_saved"] == true;
        results.push(json!({"route_id":route_id,"stop_id":request.stop_id,"response":result.0["ReaxiumResponse"]}));
        if !success {
            failed = true;
            // Continue after delivery failures, but stop after a database failure.
            if !saved {
                break;
            }
        }
    }
    if !failed {
        return response(0, "SUCCESSFUL REQUEST");
    }
    Json(
        json!({"ReaxiumResponse":{"code":3,"message":"Merged route processing incomplete. Earlier routes may be saved. Check route results and database before retrying.","object":{"routes":results}}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn perimeter_precedes_absolute_distance_and_order_breaks_ties() {
        let near = Stop {
            id: 1,
            latitude: 0.0,
            longitude: 0.001,
            radius: 1.0,
        };
        let far = Stop {
            id: 2,
            latitude: 0.0,
            longitude: 0.002,
            radius: 500.0,
        };
        assert_eq!(closest(&[near, far], 0.0, 0.0), Some(2));
        assert_eq!(
            closest(&[near, Stop { radius: 1.0, ..far }], 0.0, 0.0),
            Some(1)
        );
        assert_eq!(closest(&[near, Stop { id: 3, ..near }], 0.0, 0.0), Some(1));
        assert_eq!(closest(&[], 0.0, 0.0), None);
    }
    #[test]
    fn merged_end_preserves_situation_and_requires_it() {
        let mut input = json!({"route_ids":"7,8","device_id":42,"driver_id":1,"trace_id":"end-test","latitude":0,"longitude":0,"bus_info":"2","traffic_date":"2026-11-08 09::30::00","traffic_type_id":3,"traffic_situation_id":"2"});
        let (request, ids) = parse(input.clone(), Action::EndMerged).unwrap();
        assert_eq!(ids, [7, 8]);
        assert_eq!(request.traffic_type_id, Some(4));
        assert_eq!(request.traffic_situation_id, Some(2));
        assert_eq!(request.date().unwrap().to_string(), "2026-11-08 09:30:00");
        input
            .as_object_mut()
            .unwrap()
            .remove("traffic_situation_id");
        assert!(parse(input, Action::EndMerged).is_none());
    }

    #[test]
    fn merged_input_validates_ids_and_preserves_date() {
        let mut input = json!({"route_ids":"7, 8,7","device_id":42,"driver_id":1,"trace_id":501,"latitude":0,"longitude":0,"bus_info":2,"traffic_date":"2026-11-08 08::30::00","traffic_type_id":14,"traffic_situation_id":2});
        let (request, ids) = parse(input.clone(), Action::BeginMerged).unwrap();
        assert_eq!(ids, [7, 8, 7]);
        assert_eq!(request.traffic_type_id, Some(3));
        assert_eq!(request.traffic_situation_id, Some(1));
        assert_eq!(request.date().unwrap().to_string(), "2026-11-08 08:30:00");
        for bad in [
            json!(""),
            json!("7,"),
            json!("0"),
            json!("-1"),
            json!([7, 8]),
            json!("4294967296"),
        ] {
            input["route_ids"] = bad;
            assert!(parse(input.clone(), Action::BeginMerged).is_none());
        }
    }
}
