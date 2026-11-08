DROP TABLE traffic;
CREATE TABLE traffic (
 traffic_id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
 traffic_type_id INT, user_id BIGINT, access_id BIGINT, datetime DATETIME,
 traffic_info TEXT, device_id BIGINT, trace_id VARCHAR(100), id_route BIGINT,
 id_stop BIGINT, bus_info VARCHAR(100), latitude DOUBLE, longitude DOUBLE,
 access_situation_id BIGINT, id_stop_by_distance BIGINT, stop_number VARCHAR(100),
 stop_name VARCHAR(100), session_uid VARCHAR(100), assigned_bus VARCHAR(100)
) ENGINE=InnoDB;
INSERT INTO traffic (traffic_type_id,user_id,access_id,datetime,device_id,trace_id,id_route,bus_info,session_uid,assigned_bus) VALUES
 (1,10,100,'2020-01-01 08:00:00',42,'interrupt-test',7,'2','session-10','ORIGINAL-BUS'),
 (1,11,101,'2020-01-01 08:00:00',42,'interrupt-test',7,'2','session-11','ORIGINAL-BUS'),
 (2,11,101,'2020-01-01 09:00:00',42,'interrupt-test',7,'2','session-11','ORIGINAL-BUS'),
 (1,12,102,'2020-01-01 08:00:00',42,'other-trace',7,'2','session-12','ORIGINAL-BUS');
INSERT INTO device_activity (device_id,route_id) VALUES (42,7);
