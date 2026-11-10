# Plan: protocolo MyGraph v1 + servidor mínimo + cliente Python

## Contexto

El usuario quiere una librería Python en `python/` que hable **directamente con el servidor de MyGraph usando su protocolo**, para buscar, crear, modificar y borrar con Cypher.

Hoy ni el servidor ni el protocolo existen. v0.1 es solo embebido (README, "Not yet: server/protocol"). La especificación §9.2 describe el protocolo solo a nivel conceptual: una cabecera con versión, tipo de mensaje, request id y longitud de payload, y los mensajes HELLO, AUTH, QUERY, QUERY_RESULT, BEGIN, COMMIT, ROLLBACK, PING, PONG, ERROR y GOODBYE. No fija el formato en bytes. Para que el cliente Python tenga algo con lo que hablar hacen falta tres piezas:

1. Fijar el protocolo v1 por escrito (`docs/PROTOCOL.md`).
2. Implementarlo en Rust con un servidor mínimo (`mygraph-protocol` y `mygraph-server`, los nombres de crate de la spec §12).
3. Escribir el cliente Python, solo con la librería estándar, que habla ese protocolo por TCP.

## 1. Protocolo v1 (`docs/PROTOCOL.md`)

**Frame** (little-endian, igual que el codec de almacenamiento):

```
u8  version        = 1
u8  message_type
u32 request_id     el cliente lo asigna; la respuesta lo repite
u32 payload_len    tope de 64 MiB; si se supera → ERROR y cierre
[payload_len] payload
```

**Valores en el payload:** se reutiliza la codificación con tags de `crates/mygraph-storage/src/codec.rs` (NULL=0, BOOL=1, INT=2 i64, FLOAT=3 f64 bits, STRING=4, BYTES=5, LIST=6, MAP=7, con longitudes u32). Se añaden dos tags que solo aparecen en resultados:
- `NODE=8`: id u64, labels (u32 + strings), properties (map)
- `REL=9`: id u64, start u64, end u64, type str, properties (map)

Así se conserva la distinción int/float y los bytes, cosa que JSON pierde.

**Mensajes** (todos los payloads se codifican como un MAP de valores, lo que simplifica la evolución del protocolo):

| Tipo | Código | Dirección | Payload |
|---|---|---|---|
| HELLO | 0x01 | C→S / S→C | `{client: str}` / `{server: str, version: str, protocol: 1}` |
| AUTH | 0x02 | C→S | `{user, password}`. En v1 se acepta sin comprobar; la seguridad llega en otra fase |
| QUERY | 0x10 | C→S | `{query: str, params: map}` |
| QUERY_RESULT | 0x11 | S→C | `{columns: list<str>, rows: list<list>, stats: map<str,int>, database: str, schema: str}` |
| BEGIN / COMMIT / ROLLBACK | 0x20/0x21/0x22 | C→S | `{}` → respuesta OK |
| OK | 0x23 | S→C | `{}` |
| PING / PONG | 0x30/0x31 | | `{}` |
| ERROR | 0x7F | S→C | `{code: str, message: str, line?: int, column?: int}`, con `code` tomado de la variante de `MyGraphError` (`Syntax`, `NotFound`, `Transaction`…) |
| GOODBYE | 0x40 | C→S | `{}`, y el servidor cierra |

Reglas: el primer mensaje tiene que ser HELLO. Una petición recibe una respuesta, sin pipelining en v1. Un error en una QUERY no cierra la conexión.

## 2. Rust

### 2a. Cambio en `crates/mygraph` (necesario para el servidor)
Hoy `Database` tiene **una sola** sesión (database/schema) compartida, `Inner.session` en `crates/mygraph/src/lib.rs`. Con varias conexiones, un `USE DATABASE` de un cliente afectaría a todos. Se añade:
- Un `pub struct Session<'db>` público (o `Database::session()`) con su propio database/schema y los métodos `execute`, `begin_transaction`, `current_database`, `current_schema`.
- `run_statement`/`run_admin` reciben la sesión como argumento en vez de leer `self.session`. `Database::execute` sigue usando la sesión por defecto, así que la API embebida y los tests actuales no cambian.
- Hay que renombrar el `Session` privado actual para evitar el choque de nombres.

### 2b. `crates/mygraph-protocol` (nuevo, sin dependencias de red)
- `frame.rs`: `read_frame`/`write_frame` sobre `Read`/`Write`, con el tope de tamaño.
- `codec.rs`: codificación de valores. La parte común se mueve a `mygraph-core`, o se copia la tabla de tags, para no depender de `mygraph-storage`. Preferencia: mover `Encoder`/`Decoder` de valores a `mygraph-core::codec` y que storage lo reutilice.
- `message.rs`: `enum Message`, `encode`/`decode`, y la conversión de `MyGraphError` a ERROR.
- Tests de ida y vuelta de cada mensaje y de los frames truncados o demasiado grandes.

### 2c. `crates/mygraph-server` (nuevo binario `mygraph-server`)
- `mygraph-server start --host 127.0.0.1 --port 6660 [--data DIR]` (spec §9.1). Sin `--data` funciona en memoria.
- Usa `std::net::TcpListener` con un hilo por conexión y `Arc<Database>` compartido. Es una desviación consciente de la spec, que propone Tokio: en v0.1 las sentencias ya se ejecutan en serie, y Tokio puede llegar en la Fase 6. Se documenta en el README.
- Cada conexión tiene su `Session` y, como mucho, una `Transaction` abierta que vive en la pila del hilo, igual que hace hoy `shell()` en `crates/mygraph-cli/src/main.rs`. Al desconectarse, la transacción abierta hace rollback (ya ocurre por `Drop`).
- Limitación documentada: mientras una conexión tiene una transacción abierta, las demás se bloquean, porque la transacción guarda el mutex. Es el mismo comportamiento que en modo embebido.
- Test de integración en Rust: arranca el servidor en el puerto 0 y prueba HELLO, QUERY, BEGIN/COMMIT, ERROR y GOODBYE.

## 3. Cliente Python (`python/`)

```
python/
├── pyproject.toml          paquete "mygraph-client", módulo `mygraph`, Python ≥3.10, sin dependencias
├── README.md
├── mygraph/
│   ├── __init__.py         exporta Client, connect, Node, Relationship, Result, errores
│   ├── _protocol.py        frames (struct) + codec de valores (tags 0–9)
│   ├── types.py            Node, Relationship (dataclasses), Result (columns, rows, records(), single(), stats), Stats
│   ├── errors.py           MyGraphError → SyntaxError_/CypherSyntaxError, NotFoundError, TransactionError, ProtocolError, ConnectionError
│   ├── client.py           Client: conexión + API de alto nivel
│   └── transaction.py      Transaction (context manager)
└── tests/
    ├── test_protocol.py    ida y vuelta del codec y los frames, sin servidor
    └── test_integration.py arranca `cargo run -p mygraph-server` en un puerto libre; se salta si no hay cargo
```

**API:**

```python
from mygraph import connect

with connect("localhost", 6660) as db:
    # Cypher genérico
    r = db.run("MATCH (u:User) WHERE u.age > $a RETURN u", {"a": 18})
    for rec in r.records(): print(rec["u"].properties)

    # CRUD de nodos (genera Cypher con parámetros; las etiquetas y claves van con backticks)
    alice = db.create_node("User", {"name": "Alice", "age": 25})          # -> Node
    users = db.find_nodes("User", {"name": "Alice"}, limit=10)            # -> list[Node]
    node  = db.get_node(alice.id)                                         # MATCH (n) WHERE id(n) = $id
    db.update_node(alice.id, {"age": 26})                                 # SET n += $props
    db.update_node(alice.id, {"age": 26}, replace=True)                   # SET n = $props
    db.delete_node(alice.id, detach=True)                                 # DETACH DELETE

    # Relaciones
    rel = db.create_relationship(alice.id, bob.id, "KNOWS", {"since": 2020})
    db.find_relationships("KNOWS", props=None, start_id=alice.id)
    db.update_relationship(rel.id, {...}); db.delete_relationship(rel.id)

    # Transacciones
    with db.transaction() as tx:
        tx.run("CREATE (:User {name: $n})", {"n": "Carol"})
    # commit al salir; rollback si hay excepción

    db.use_database("ecommerce"); db.use_schema("public"); db.ping()
```

- Las funciones de ayuda usan solo lo que el parser ya soporta: `id()`, backticks (lexer.rs), `SET +=`, `DETACH DELETE`. Sin MERGE (no existe en v0.1), así que no hay upsert.
- Los nombres de etiqueta y tipo se escapan con backticks, duplicando las backticks internas, nunca interpolando valores. Los valores siempre van como parámetros.
- Los `int` de Python fuera del rango i64 lanzan `ValueError`. `bytes` va a BYTES y `dict` a MAP, con claves str obligatorias.
- `Client` no es thread-safe (una conexión, una petición a la vez). Se documenta.

## 4. Documentación
- `docs/PROTOCOL.md` (nuevo), `python/README.md`.
- README raíz: tabla de estado (servidor y protocolo v1, cliente Python), comandos `mygraph-server start` y uso de Python, y la desviación std-threads frente a Tokio.

## Verificación
1. `cargo test --workspace`: los tests existentes siguen en verde (la API embebida no cambia), más los nuevos de protocol y server.
2. `cargo run -p mygraph-server -- start --port 6660 --data /tmp/x` y luego `cd python && python3 -m pytest`: codec, CRUD completo, transacciones con commit/rollback, error de sintaxis con línea/columna, aislamiento de `USE DATABASE` entre dos conexiones, rollback al desconectarse.
3. Prueba manual: script de ejemplo `python/examples/crud.py` contra el servidor en marcha.
4. Commit en una rama nueva (no en `main`), con `git pull --rebase` antes de commitear.
