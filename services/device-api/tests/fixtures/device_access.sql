DROP TABLE IF EXISTS traffic, stops, user_access_data, bus_routes_relationship, bus, messages, traffic_type, routes, route_type, users;
CREATE TABLE user_access_data (user_access_data_id INT PRIMARY KEY,user_id INT,access_type_id INT,status_id INT);
INSERT INTO user_access_data VALUES (100,1,3,2);
CREATE TABLE bus (bus_id INT PRIMARY KEY,bus_number VARCHAR(100));
INSERT INTO bus VALUES (1,'ROUTE-BUS');
CREATE TABLE bus_routes_relationship (bus_id INT,id_route INT,last_update DATETIME);
INSERT INTO bus_routes_relationship VALUES (1,7,'2026-01-01');
CREATE TABLE stops (id_stop INT PRIMARY KEY,stop_number VARCHAR(100),stop_name VARCHAR(100));
INSERT INTO stops VALUES (9,'ORIGINAL','Original'),(10,'DISTANCE','Distance');
CREATE TABLE traffic (
 traffic_id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
 traffic_type_id INT NOT NULL,user_id INT NOT NULL,access_id INT NOT NULL,
 device_id INT NOT NULL,datetime TIMESTAMP NOT NULL,traffic_info VARCHAR(2500),
 trace_id VARCHAR(150) NOT NULL,id_route INT,id_stop INT,id_stop_by_distance INT,
 bus_info VARCHAR(45),latitude VARCHAR(45),longitude VARCHAR(45),access_situation_id INT,
 assigned_bus VARCHAR(150),stop_number VARCHAR(100),stop_name VARCHAR(100),last_update DATETIME,
 UNIQUE KEY UQ_traffic(trace_id,datetime,traffic_type_id,device_id,bus_info,user_id)
) ENGINE=InnoDB;
CREATE TABLE messages (
 id BIGINT AUTO_INCREMENT PRIMARY KEY,title TEXT,message TEXT,data LONGTEXT,stack_trace LONGTEXT,
 topic VARCHAR(100),device_id VARCHAR(100),status VARCHAR(100),created_at DATETIME,
 created_by VARCHAR(100),last_modified_at DATETIME,last_modified_by VARCHAR(100)
);

CREATE TABLE traffic_type (traffic_type_id INT PRIMARY KEY,traffic_type_name VARCHAR(100));
INSERT INTO traffic_type VALUES (1,'IN');
CREATE TABLE routes (id_route INT PRIMARY KEY,route_number VARCHAR(50),route_name VARCHAR(100),route_type INT);
INSERT INTO routes VALUES (7,'7','Route 7',1);
CREATE TABLE route_type (route_type_id INT PRIMARY KEY,route_type_name VARCHAR(100));
INSERT INTO route_type VALUES (1,'AM');
CREATE TABLE users (user_id INT PRIMARY KEY,first_name VARCHAR(100),first_last_name VARCHAR(100),document_id VARCHAR(100),user_photo VARCHAR(100),birthdate DATE);
INSERT INTO users VALUES (1,'Test','Passenger','TEST','photo','2015-01-01');
