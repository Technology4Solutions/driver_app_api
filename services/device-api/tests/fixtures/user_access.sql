-- Isolated fixture matching the production constraints relevant to passenger access.
DROP TABLE IF EXISTS traffic,traffic_view,user_access_data;
CREATE TABLE user_access_data (
 user_access_data_id INT PRIMARY KEY,user_id BIGINT,access_type_id INT,status_id INT
);
INSERT INTO user_access_data VALUES (100,1,3,2);
CREATE TABLE traffic (
 traffic_id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
 traffic_type_id INT NOT NULL,user_id BIGINT NOT NULL,access_id INT NOT NULL,
 datetime TIMESTAMP NOT NULL,traffic_info VARCHAR(2500),device_id BIGINT NOT NULL,
 trace_id VARCHAR(150) NOT NULL,id_route BIGINT,id_stop BIGINT,id_stop_by_distance BIGINT,
 bus_info VARCHAR(45),latitude VARCHAR(45),longitude VARCHAR(45),
 access_situation_id INT,session_uid INT NOT NULL DEFAULT 0,assigned_bus VARCHAR(150),
 stop_number VARCHAR(100),stop_name VARCHAR(100),
 UNIQUE KEY UQ_traffic (trace_id,datetime,traffic_type_id,device_id,bus_info,user_id)
) ENGINE=InnoDB;
CREATE TABLE traffic_view (traffic_id BIGINT UNSIGNED,trace_id VARCHAR(150),traffic_type_id INT);
CREATE TRIGGER traffic_insert AFTER INSERT ON traffic FOR EACH ROW
 INSERT INTO traffic_view VALUES (NEW.traffic_id,NEW.trace_id,NEW.traffic_type_id);
INSERT INTO traffic_type VALUES (1,'BOARDING'),(2,'DROP OFF');
INSERT INTO stops (id_stop,stop_name,stop_number,stop_latitude,stop_longitude)
 VALUES (10,'Distance stop','STOP10',27.2,-80.3);
