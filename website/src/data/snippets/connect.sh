pip install mygraph-client
mygraph-client "CREATE (a:User {name: 'Alice'})-[:KNOWS]->(b:User {name: 'Bob'});"
mygraph-client "MATCH (a)-[:KNOWS]->(b) RETURN a.name, b.name;"
