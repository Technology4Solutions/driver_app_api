-- Extend device_status.sql only inside the isolated integration test database.
DROP TABLE IF EXISTS bus_routes_relationship, traffic_type, route_type, route_trigger_audit;
ALTER TABLE bus ADD bus_number VARCHAR(100);
UPDATE bus SET bus_number='ROUTE-BUS';
INSERT INTO bus (bus_id,gps_serial,bus_number) VALUES (2,'gps-request','REQUEST-BUS');
CREATE TABLE bus_routes_relationship (bus_id BIGINT, id_route BIGINT, last_update DATETIME);
INSERT INTO bus_routes_relationship VALUES (1,7,'2026-01-01 00:00:00');
ALTER TABLE users ADD birthdate DATE;
ALTER TABLE stops ADD stop_number VARCHAR(100), ADD stop_latitude DECIMAL(12,8), ADD stop_longitude DECIMAL(12,8);
UPDATE stops SET stop_number='STOP9', stop_latitude=27.1975, stop_longitude=-80.2528;
CREATE TABLE traffic_type (traffic_type_id INT PRIMARY KEY, traffic_type_name VARCHAR(100));
INSERT INTO traffic_type VALUES (3,'ROUTE START');
CREATE TABLE route_type (route_type_id INT PRIMARY KEY, route_type_name VARCHAR(100));
INSERT INTO route_type VALUES (1,'AM');
DROP TABLE device_traffic;
CREATE TABLE device_traffic (
 device_traffic_id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
 device_id BIGINT, user_id BIGINT, id_route BIGINT, id_stop BIGINT,
 stop_number VARCHAR(100), stop_name VARCHAR(100), latitude DOUBLE, longitude DOUBLE,
 traffic_type_id INT, traffic_situation_id INT, traffic_date TIMESTAMP,
 trace_id VARCHAR(100), bus_info VARCHAR(100), assigned_bus VARCHAR(100)
) ENGINE=InnoDB;
CREATE TABLE route_trigger_audit (device_traffic_id BIGINT UNSIGNED) ENGINE=InnoDB;
CREATE TRIGGER device_traffic_insert AFTER INSERT ON device_traffic FOR EACH ROW INSERT INTO route_trigger_audit VALUES (NEW.device_traffic_id);
