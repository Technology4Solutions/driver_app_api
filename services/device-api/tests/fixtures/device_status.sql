-- Minimal isolated schema for the fields read/written by the legacy endpoint.
-- This is a test fixture, not a production migration.
DROP TABLE IF EXISTS routes_stops_relationship, device_activity, device_status, device_location, reaxium_device,
 bus, bus_device_relationship, users, user_type, business, business_type, address,
 device_traffic, routes, route_users, business_routes, route_group_relationship,
 route_group, stops, routesStopsRelationship, traffic, access_situation;
CREATE TABLE reaxium_device (device_id BIGINT UNSIGNED PRIMARY KEY, device_serial VARCHAR(100), status_id INT, device_name VARCHAR(100), device_description VARCHAR(100), device_token VARCHAR(100));
CREATE TABLE device_location (device_location_id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY, device_id BIGINT, user_id BIGINT, latitude DECIMAL(12,8), longitude DECIMAL(12,8), speed DOUBLE, bearing DOUBLE, accuracy DOUBLE, lag_time DOUBLE, date_location DATETIME);
CREATE TABLE device_status (device_status_id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY, device_id BIGINT, business_id BIGINT, temperature DOUBLE, battery_power_level DOUBLE, is_charging TINYINT, device_location_id BIGINT UNSIGNED, last_sync DATETIME, last_status_update DATETIME, installed_app_version VARCHAR(100), bus_info VARCHAR(100));
CREATE TABLE device_activity (device_activity_id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY, device_id BIGINT, route_id BIGINT, users_in_track TEXT, activity_date DATETIME);
CREATE TABLE bus (bus_id BIGINT PRIMARY KEY, gps_serial VARCHAR(100));
CREATE TABLE bus_device_relationship (bus_id BIGINT, device_id BIGINT);
CREATE TABLE users (user_id BIGINT PRIMARY KEY, status_id INT, first_name VARCHAR(100), first_last_name VARCHAR(100), document_id VARCHAR(100), user_photo VARCHAR(100), business_id BIGINT, user_type_id BIGINT, password VARCHAR(100));
CREATE TABLE user_type (user_type_id BIGINT PRIMARY KEY, user_type_name VARCHAR(100));
CREATE TABLE business (business_id BIGINT PRIMARY KEY, business_type_id BIGINT, address_id BIGINT);
CREATE TABLE business_type (business_type_id BIGINT PRIMARY KEY);
CREATE TABLE address (address_id BIGINT PRIMARY KEY);
CREATE TABLE device_traffic (device_id BIGINT, id_route BIGINT, traffic_date DATETIME);
CREATE TABLE routes (id_route BIGINT PRIMARY KEY, status_id INT, route_number VARCHAR(100), route_name VARCHAR(100), route_address VARCHAR(100), route_type INT, start_time TIME, end_time TIME, shuttle_id BIGINT, overview_polyline TEXT);
CREATE TABLE route_users (id_route BIGINT, user_id BIGINT);
CREATE TABLE business_routes (id_route BIGINT, business_id BIGINT);
CREATE TABLE route_group_relationship (route_id BIGINT, route_group_id BIGINT);
CREATE TABLE route_group (id_route_group BIGINT PRIMARY KEY, group_name VARCHAR(100), status_id INT);
CREATE TABLE stops (id_stop BIGINT PRIMARY KEY, stop_name VARCHAR(100));
CREATE TABLE routesStopsRelationship (id_route BIGINT, id_stop BIGINT);
CREATE TABLE traffic (user_id BIGINT, id_stop BIGINT, id_stop_by_distance BIGINT, datetime DATETIME, access_situation_id BIGINT, trace_id VARCHAR(100));
CREATE TABLE access_situation (access_situation_id BIGINT PRIMARY KEY, access_situation_name VARCHAR(100));
INSERT INTO reaxium_device VALUES (42,'registered-device',1,'Test','Test device','test-token'), (43,'inactive-device',0,'Inactive','Inactive',NULL);
INSERT INTO bus VALUES (1,'gps-42');
INSERT INTO bus_device_relationship VALUES (1,42);
INSERT INTO users VALUES (1,1,'Test','Driver','doc-1',NULL,1,1,'must-not-be-broadcast');
INSERT INTO user_type VALUES (1,'Driver');
INSERT INTO business VALUES (1,1,1),(2,3,1);
INSERT INTO business_type VALUES (1),(3);
INSERT INTO address VALUES (1);
INSERT INTO device_traffic VALUES (42,7,'2026-10-20 10:00:00');
INSERT INTO routes VALUES (7,1,'7','Test Route','Test Address',1,'08:00:00','09:00:00',1,'encoded-polyline');
INSERT INTO route_users VALUES (7,1);
INSERT INTO business_routes VALUES (7,1);
INSERT INTO route_group_relationship VALUES (7,1);
INSERT INTO route_group VALUES (1,'Test Group',1);
INSERT INTO stops VALUES (9,'Test stop');
INSERT INTO routesStopsRelationship VALUES (7,9);
INSERT INTO traffic VALUES (1,9,9,'2026-10-20 09:00:00',1,'trace-1');
INSERT INTO access_situation VALUES (1,'Boarded');
