INSERT INTO traffic (
    traffic_type_id,
    user_id,
    access_id,
    datetime,
    traffic_info,
    device_id,
    trace_id,
    id_route,
    id_stop,
    bus_info,
    latitude,
    longitude,
    access_situation_id,
    id_stop_by_distance
    /* optional_columns */
)
SELECT
    2 AS traffic_type_id,
    t.user_id,
    t.access_id,
    CONVERT_TZ(NOW(), 'UTC', 'America/New_York') AS datetime,
    'Automatic drop off due to route interruption' AS traffic_info,
    t.device_id,
    t.trace_id,
    t.id_route,
    ? AS id_stop,
    t.bus_info,
    ? AS latitude,
    ? AS longitude,
    20 AS access_situation_id,
    ? AS id_stop_by_distance
    /* optional_values */
FROM traffic t
         LEFT JOIN traffic t2 ON t.user_id = t2.user_id
    AND t.trace_id = t2.trace_id
    AND t2.traffic_type_id = 2
    AND t2.datetime > t.datetime
         LEFT JOIN stops s ON s.id_stop = ?
WHERE t.trace_id = ?
  AND t.traffic_type_id = 1
  AND t2.traffic_id IS NULL;
