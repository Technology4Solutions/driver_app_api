SELECT JSON_OBJECT('route_id',ru.id_route,'passenger_id',u.user_id,'access_type',MIN(ua.access_type_id))
FROM route_users ru
JOIN users u ON u.user_id=ru.user_id AND u.status_id=1 AND u.user_type_id=2
JOIN user_access_data ua ON ua.user_id=u.user_id AND ua.access_type_id>0
WHERE ru.id_route IN (__ROUTES__)
  AND NOT EXISTS (SELECT 1 FROM traffic t WHERE t.user_id=u.user_id
                  AND t.datetime >= '__DATE__ 00:00:00'
                  AND t.datetime < DATE_ADD('__DATE__ 00:00:00', INTERVAL 1 DAY))
GROUP BY ru.id_route,u.user_id
ORDER BY ru.id_route,u.user_id;
