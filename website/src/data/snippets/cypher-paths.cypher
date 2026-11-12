// What reads this field, up to 8 hops away?
MATCH (f:Field {name: 'customer_id'})<-[:READS|CALLS*1..8]-(p:Program)
RETURN DISTINCT p.name AS program

// Shortest path between two bound nodes
MATCH (a:Job {name: 'nightly'}), (b:Table {name: 'orders'}),
      p = shortestPath((a)-[*..10]-(b))
RETURN length(p) AS hops, nodes(p) AS steps

// Lightest path: the smallest sum of a relationship property
MATCH (a:Job {name: 'nightly'}), (b:Table {name: 'orders'}),
      p = shortestPath((a)-[*]-(b), {weight: 'cost'})
UNWIND relationships(p) AS r RETURN length(p) AS hops, sum(r.cost) AS cost
