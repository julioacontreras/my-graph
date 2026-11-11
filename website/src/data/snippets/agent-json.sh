mygraph query --host localhost --json "MATCH (u:User)-[r:KNOWS]->(f) RETURN u.name AS user, f.name AS friend"
# [{"user":"Alice","friend":"Bob"}]
