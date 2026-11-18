-- Attendance services persist supplied references; they do not require an
-- activities_users assignment. Prefer related references, then existing test
-- reference rows, without creating data or changing business relationships.
WITH attendance_candidates AS (
    SELECT u.business_id, u.user_id,
           (SELECT MIN(ua.access_type_id) FROM user_access_data ua
            WHERE ua.user_id = u.user_id AND ua.access_type_id > 0) AS access_type_id,
           COALESCE(
               (SELECT MIN(a.activity_id) FROM activities_users au JOIN activities a ON a.activity_id = au.activity_id WHERE au.user_id = u.user_id),
               (SELECT MIN(a.activity_id) FROM business_activities ba JOIN activities a ON a.activity_id = ba.activity_id WHERE ba.business_id = u.business_id),
               (SELECT MIN(activity_id) FROM activities)
           ) AS activity_id,
           COALESCE(
               (SELECT MIN(p.place_id) FROM activities_users au JOIN places p ON p.place_id = au.place_id WHERE au.user_id = u.user_id),
               (SELECT MIN(p.place_id) FROM business_places bp JOIN places p ON p.place_id = bp.place_id WHERE bp.business_id = u.business_id),
               (SELECT MIN(place_id) FROM places)
           ) AS place_id
    FROM users u WHERE u.status_id = 1
), ranked AS (
    SELECT attendance_candidates.*,
           ROW_NUMBER() OVER (PARTITION BY business_id ORDER BY access_type_id IS NULL, user_id) AS choice
    FROM attendance_candidates
)
SELECT JSON_OBJECT('business_id', business_id, 'attendance_user_id', user_id,
    'attendance_activity_id', activity_id, 'attendance_place_id', place_id,
    'attendance_access_type', access_type_id,
    'attendance_traffic_type', (SELECT traffic_type_id FROM traffic_type WHERE traffic_type_id = 1),
    'attendance_device_type', (SELECT traffic_type_id FROM traffic_type WHERE traffic_type_id = 21))
FROM ranked WHERE choice = 1;
