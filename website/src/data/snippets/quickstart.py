import mygraph

with mygraph.connect("localhost", 6660) as db:
    db.run("CREATE (:User {name: 'Alice'})"
           "-[:KNOWS]->(:User {name: 'Bob'})")
    rows = db.run("MATCH (a)-[:KNOWS]->(b) "
                  "RETURN a.name AS a, b.name AS b")
    for row in rows:
        print(row)  # {'a': 'Alice', 'b': 'Bob'}
# That's it. Durable on disk. MIT licensed.
