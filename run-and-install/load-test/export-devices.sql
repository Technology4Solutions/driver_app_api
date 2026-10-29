WITH route_stops AS (
    SELECT rel.id_route, MIN(s.id_stop) AS start_stop, MAX(s.id_stop) AS end_stop
    FROM routes_stops_relationship rel
    JOIN stops s ON s.id_stop=rel.id_stop AND s.status_id=1
    WHERE s.stop_latitude BETWEEN -90 AND 90 AND s.stop_longitude BETWEEN -180 AND 180
    GROUP BY rel.id_route
), candidates AS (
    SELECT d.device_id, d.device_serial AS device_imei, u.business_id, r.id_route AS route_id, u.user_id AS driver_id, b.bus_id,
           ss.id_stop AS start_stop_id, es.id_stop AS end_stop_id,
           ss.stop_latitude AS start_latitude, ss.stop_longitude AS start_longitude,
           es.stop_latitude AS end_latitude, es.stop_longitude AS end_longitude,
           ROW_NUMBER() OVER (PARTITION BY d.device_id ORDER BY bd.last_update DESC, br.last_update DESC, dr.last_update DESC, b.bus_id,r.id_route,u.user_id) AS choice
    FROM reaxium_device d
    JOIN bus_device_relationship bd ON bd.device_id=d.device_id AND bd.status_id=1
    JOIN bus b ON b.bus_id=bd.bus_id AND b.bus_number IS NOT NULL
    JOIN bus_routes_relationship br ON br.bus_id=b.bus_id
    JOIN routes r ON r.id_route=br.id_route AND r.status_id=1
    JOIN route_type rt ON rt.route_type_id=r.route_type
    JOIN bus_driver_relationship dr ON dr.bus_id=b.bus_id AND dr.relation_type_id=19
    JOIN users u ON u.user_id=dr.user_id AND u.status_id=1
    JOIN business business ON business.business_id=u.business_id
    JOIN route_stops rs ON rs.id_route=r.id_route
    JOIN stops ss ON ss.id_stop=rs.start_stop
    JOIN stops es ON es.id_stop=rs.end_stop
    WHERE d.status_id=1
)
SELECT JSON_OBJECT('device_id',device_id,'device_imei',device_imei,'business_id',business_id,'route_id',route_id,'driver_id',driver_id,
 'bus_info',CAST(bus_id AS CHAR),'start_stop_id',start_stop_id,'end_stop_id',end_stop_id,
 'start_latitude',start_latitude,'start_longitude',start_longitude,
 'end_latitude',end_latitude,'end_longitude',end_longitude)
FROM candidates WHERE choice=1 ORDER BY device_id LIMIT __LIMIT__;
