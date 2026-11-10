# MyGraph --- Especificación técnica y diseño de arquitectura

**Versión:** 1.0\
**Estado:** Diseño arquitectónico\
**Lenguaje principal:** Rust\
**Modelo:** Property Graph\
**Almacenamiento:** Híbrido (RAM + disco)\
**Compatibilidad de consultas:** Cypher, con compatibilidad completa
como objetivo\
**Modos de ejecución:** Servidor y embebido\
**Filosofía:** Equilibrio entre rendimiento, funcionalidades y
mantenibilidad

------------------------------------------------------------------------

## 1. Visión y objetivos

MyGraph será un sistema gestor de bases de datos de grafos (GDBMS)
independiente, escrito en Rust y con motor, almacenamiento y protocolo
propios.

Objetivos funcionales:

-   Crear y administrar múltiples bases de datos y schemas.
-   Crear, consultar, modificar y eliminar nodos y relaciones.
-   Consultar grafos mediante Cypher.
-   Ejecutar transacciones ACID.
-   Mantener datos en memoria y persistirlos en disco.
-   Ejecutarse como servidor independiente.
-   Integrarse como librería Rust embebida.
-   Importar y exportar datos en Cypher.
-   Administrar usuarios, permisos, índices y restricciones.
-   Admitir conexiones y consultas concurrentes.

### Principios de diseño

1.  **Rust nativo:** no depender de otro motor de bases de datos.
2.  **Modularidad:** componentes separados mediante Cargo Workspace.
3.  **Core independiente:** el motor funciona sin servidor ni runtime de
    red.
4.  **Server-ready:** el servidor utiliza exactamente el mismo motor que
    Embedded.
5.  **Abstracción de almacenamiento:** el motor no depende de la
    implementación física del almacenamiento.
6.  **Cypher-first:** el lenguaje de consultas es un componente central.
7.  **Durabilidad:** los datos confirmados deben sobrevivir a cierres
    inesperados, según la política de sincronización configurada.
8.  **Extensibilidad:** permitir añadir capacidades sin rediseñar el
    sistema completo.

> La compatibilidad completa con Cypher es un objetivo de conformidad
> verificable, no una promesa de que todas las extensiones de Neo4j y
> Memgraph estarán disponibles desde la primera versión.

------------------------------------------------------------------------

## 2. Arquitectura general

MyGraph tendrá las siguientes capas:

1.  **Client Layer**
    -   CLI Client.
    -   API Rust para Embedded.
2.  **Server Layer**
    -   TCP Protocol.
    -   Connection Manager.
    -   Authentication y Authorization.
    -   Session Manager.
    -   Query Handler.
3.  **MyGraph Core**
    -   Cypher Lexer y Parser.
    -   AST.
    -   Semantic Analyzer.
    -   Query Planner y Optimizer.
    -   Query Executor.
    -   Graph Engine.
    -   Schema/Catalog Manager.
    -   Index Manager.
    -   Transaction Manager.
4.  **Storage Engine**
    -   Memory Store y Buffer Pool.
    -   Disk Store.
    -   WAL.
    -   Checkpoint Manager.
    -   Recovery.
5.  **Physical Storage**
    -   Archivos de bases de datos.
    -   Archivos de índices.
    -   Logs de transacciones.
    -   Metadatos.

### Regla arquitectónica fundamental

Tanto el CLI como la API embebida utilizan el mismo `mygraph-core`. No
habrá dos motores de ejecución distintos.

La capa de servidor es opcional y solo se utiliza en modalidad
cliente-servidor.

------------------------------------------------------------------------

## 3. Modelo de datos

MyGraph utilizará el modelo **Property Graph**.

Elementos principales:

-   **Nodes:** entidades del grafo.
-   **Relationships:** conexiones dirigidas entre nodos.
-   **Labels:** clasificación de nodos.
-   **Relationship Types:** clasificación de relaciones.
-   **Properties:** pares clave-valor.
-   **Paths:** secuencias de nodos y relaciones.

### 3.1 Nodes

Campos conceptuales:

  Campo        Tipo                    Descripción
  ------------ ----------------------- -------------------------
  id           u64                     Identificador interno
  schema_id    u32                     Schema al que pertenece
  labels       Vec`<LabelId>`{=html}   Etiquetas
  properties   PropertyMap             Propiedades
  created_at   u64                     Fecha de creación
  updated_at   u64                     Última modificación

Ejemplo:

``` cypher
CREATE (u:User {
    name: "Alice",
    age: 25,
    active: true
});
```

### 3.2 Relationships

  Campo               Tipo          Descripción
  ------------------- ------------- -----------------------
  id                  u64           Identificador interno
  schema_id           u32           Schema
  start_node          u64           Nodo origen
  end_node            u64           Nodo destino
  relationship_type   u32           Tipo de relación
  properties          PropertyMap   Propiedades

Las relaciones serán dirigidas. Las consultas podrán recorrerlas en la
dirección especificada o en ambas, según el patrón Cypher.

Ejemplo:

``` cypher
CREATE (a:User {name: "Alice"})
CREATE (b:User {name: "Bob"})
CREATE (a)-[:KNOWS {since: 2024}]->(b);
```

### 3.3 Tipos de propiedades

Representación conceptual inicial:

``` rust
pub enum Value {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
}
```

Se podrán añadir tipos temporales, espaciales y otros tipos específicos
de Cypher.

Los valores persistidos tendrán una representación binaria versionada,
independiente de la representación interna de Rust. Se deberá definir
explícitamente la semántica de igualdad, ordenación, conversión y
`NULL`.

------------------------------------------------------------------------

## 4. Bases de datos y schemas

MyGraph distinguirá entre **database** y **schema**.

Jerarquía:

``` text
MyGraph Instance
├── Database: ecommerce
│   ├── Schema: products
│   └── Schema: orders
└── Database: social
    └── Schema: public
```

Cada database tendrá:

-   Identificador único.
-   Configuración independiente.
-   Schemas.
-   Índices y restricciones.
-   Datos.
-   Transacciones.

Cada schema tendrá sus propias etiquetas, tipos de relaciones y
restricciones.

Ejemplo conceptual de administración:

``` cypher
CREATE DATABASE ecommerce;
USE DATABASE ecommerce;

CREATE SCHEMA products;
USE SCHEMA products;

CREATE (:Product {
    name: "Keyboard",
    price: 99.99
});
```

`CREATE DATABASE`, `USE DATABASE`, `CREATE SCHEMA` y `USE SCHEMA` se
definirán como extensiones administrativas de MyGraph. No deben
presentarse como sintaxis universal de Cypher.

Se implementará un catálogo de metadatos para consultar bases de datos,
schemas, índices y restricciones.

------------------------------------------------------------------------

## 5. Cypher Engine

El motor de consultas tendrá estas fases:

1.  **Lexer:** convierte el texto en tokens.
2.  **Parser:** valida la sintaxis y genera el AST.
3.  **Semantic Analyzer:** valida variables, scopes, tipos, funciones y
    contexto de base de datos/schema.
4.  **Logical Planner:** genera el plan lógico.
5.  **Optimizer:** transforma el plan para reducir el coste estimado.
6.  **Physical Planner:** selecciona operadores físicos.
7.  **Executor:** ejecuta los operadores.
8.  **Result Set:** devuelve resultados o errores.

### 5.1 Funcionalidades Cypher

  Grupo               Funcionalidades
  ------------------- --------------------------------------
  Data Query          MATCH, OPTIONAL MATCH, WHERE, RETURN
  Data Creation       CREATE, MERGE
  Data Modification   SET, REMOVE
  Data Deletion       DELETE, DETACH DELETE
  Projection          WITH, DISTINCT
  Sorting             ORDER BY, SKIP, LIMIT
  Expressions         CASE, operadores y funciones
  Aggregations        COUNT, SUM, AVG, MIN, MAX, COLLECT
  Lists               UNWIND y operaciones de listas
  Paths               Patrones y caminos
  Subqueries          CALL, EXISTS
  Administration      SHOW, CREATE INDEX, DROP INDEX
  Transactions        BEGIN, COMMIT, ROLLBACK

También se incluirán parámetros, alias, funciones escalares y agregadas,
manejo de `NULL` y errores con posición de línea y columna.

### 5.2 Perfiles de compatibilidad

Se propone organizar las compatibilidades en perfiles:

-   `opencypher`: comportamiento basado en la especificación openCypher.
-   `neo4j`: extensiones y comportamientos compatibles con Neo4j.
-   `memgraph`: extensiones y comportamientos compatibles con Memgraph.
-   `mygraph`: extensiones propias.

La base de conformidad será openCypher y su Technology Compatibility Kit
(TCK). Las diferencias entre perfiles se documentarán explícitamente. No
se asumirá compatibilidad con extensiones sin pruebas.

### 5.3 Query Planner

Ejemplo:

``` cypher
MATCH (u:User)-[:KNOWS]->(f:User)
WHERE u.name = "Alice"
RETURN f.name;
```

Plan conceptual:

``` text
NodeIndexSeek(User, name = "Alice")
    |
ExpandOutgoing(KNOWS)
    |
Filter
    |
Project(f.name)
    |
Return
```

Operadores físicos iniciales:

-   NodeScan.
-   NodeIndexSeek.
-   RelationshipScan.
-   Expand.
-   ExpandInto.
-   Filter.
-   Project.
-   Sort.
-   Limit.
-   Aggregate.
-   CreateNode.
-   CreateRelationship.
-   Delete.
-   SetProperty.

La ejecución utilizará operadores iterativos y streaming de resultados
cuando sea posible.

------------------------------------------------------------------------

## 6. Storage Engine: almacenamiento híbrido

El almacenamiento será híbrido: el disco constituye la fuente
persistente de verdad y la memoria proporciona acceso rápido mediante
cachés y buffers. No se duplicarán indiscriminadamente todos los datos
en RAM.

### 6.1 Memory Layer

-   Page Cache.
-   Index Cache.
-   Query Cache, si se habilita y se define una política de invalidación
    segura.
-   Transaction Workspace.
-   Buffer Pool.

### 6.2 Persistent Layer

-   Node Store.
-   Relationship Store.
-   Property Store.
-   Label Store.
-   Adjacency Store.
-   Index Store.
-   WAL.

### 6.3 Organización de archivos

``` text
data/
├── system/
│   ├── catalog.dat
│   └── users.dat
├── databases/
│   ├── ecommerce/
│   │   ├── metadata.dat
│   │   ├── nodes.dat
│   │   ├── relationships.dat
│   │   ├── properties.dat
│   │   ├── adjacency.dat
│   │   ├── indexes/
│   │   ├── wal/
│   │   └── checkpoints/
│   └── social/
│       ├── metadata.dat
│       ├── nodes.dat
│       ├── relationships.dat
│       ├── properties.dat
│       ├── adjacency.dat
│       ├── indexes/
│       ├── wal/
│       └── checkpoints/
└── config.toml
```

### 6.4 Page Manager

Se propone un tamaño de página inicial configurable de 16 KiB.

Cada página tendrá:

-   Page ID.
-   Database ID.
-   Page type.
-   Version.
-   Checksum.
-   Payload.
-   Información de ocupación.

El Page Manager será responsable de leer y escribir páginas, asignarlas,
gestionar páginas sucias y coordinar el buffer pool.

El formato físico deberá especificar endianness, alineación, versionado,
checksums y reglas de compatibilidad.

### 6.5 WAL y recuperación

Flujo de escritura:

1.  Comienza una transacción.
2.  Se generan cambios.
3.  Los cambios se registran en el WAL.
4.  Se escribe el registro de commit.
5.  Se sincroniza el WAL según la política de durabilidad.
6.  Las páginas de datos se escriben posteriormente.
7.  Se ejecutan checkpoints periódicos.

Al reiniciar:

-   Identificar el último checkpoint válido.
-   Leer los registros posteriores.
-   Reaplicar los cambios de transacciones confirmadas que no estén
    reflejados en las páginas.
-   Descartar los cambios no confirmados.
-   Reconstruir un estado consistente.

Se utilizarán checksums para detectar corrupción y una política
explícita de `fsync` para commits durables.

### 6.6 Política de caché

La primera implementación podrá utilizar LRU. Se podrán evaluar Clock u
otras variantes mediante benchmarks.

Requisitos:

-   Tamaño máximo configurable.
-   Evicción de páginas no fijadas.
-   Gestión de páginas sucias.
-   Escritura diferida.
-   Métricas de hit/miss.
-   Protección frente a expulsar páginas que estén en uso.

Configuración inicial:

``` toml
[storage]
mode = "hybrid"
page_size = 16384
cache_size_mb = 1024
wal_enabled = true
checkpoint_interval_secs = 300
sync_on_commit = true
```

------------------------------------------------------------------------

## 7. Transaction Manager y concurrencia

MyGraph implementará transacciones ACID.

  Propiedad     Implementación prevista
  ------------- --------------------------------------
  Atomicity     WAL y rollback
  Consistency   Validación de constraints
  Isolation     MVCC y validación de conflictos
  Durability    WAL sincronizado según configuración

### 7.1 MVCC

Se utilizará Multi-Version Concurrency Control.

Cada modificación generará una nueva versión lógica del registro. Las
versiones incluirán información transaccional de creación y eliminación
para determinar su visibilidad.

Objetivos:

-   Lecturas concurrentes.
-   Escrituras concurrentes cuando no haya conflictos.
-   Lecturas consistentes.
-   Evitar bloquear todas las consultas durante una escritura.
-   Recolección segura de versiones antiguas.

La garbage collection respetará el snapshot más antiguo que siga activo.

### 7.2 API conceptual

``` rust
pub trait Transaction {
    fn commit(&mut self) -> Result<(), MyGraphError>;
    fn rollback(&mut self) -> Result<(), MyGraphError>;
}
```

Ejemplo:

``` rust
let mut tx = db.begin_transaction()?;

tx.execute(
    "CREATE (:User {name: 'Alice'})",
    params! {}
)?;

tx.commit()?;
```

La API final controlará el ciclo de vida de la transacción e impedirá
commits dobles o modificaciones después del cierre.

### 7.3 Aislamiento

Objetivo inicial: `READ COMMITTED` y `SNAPSHOT ISOLATION`, con detección
y resolución de conflictos de escritura.

La serializabilidad no se dará por garantizada por usar MVCC: requerirá
un diseño y pruebas específicas.

------------------------------------------------------------------------

## 8. Index Manager

Los índices iniciales serán:

  Índice                    Estructura propuesta
  ------------------------- ----------------------
  Node ID                   Acceso directo
  Relationship ID           Acceso directo
  Property Index            B+ Tree
  Composite Index           B+ Tree
  Unique Index              B+ Tree + constraint
  Label Index               Posting lists
  Relationship Type Index   Posting lists

Los índices secundarios serán transaccionales. Un commit no podrá dejar
un índice desactualizado respecto a los registros.

Ejemplos:

``` cypher
CREATE INDEX user_email_idx
FOR (u:User)
ON (u.email);

CREATE INDEX product_category_price_idx
FOR (p:Product)
ON (p.category, p.price);

CREATE CONSTRAINT user_email_unique
FOR (u:User)
REQUIRE u.email IS UNIQUE;

SHOW INDEXES;
DROP INDEX user_email_idx;
```

Full-text y vector indexes quedan para versiones posteriores.

------------------------------------------------------------------------

## 9. Server y Client

### 9.1 MyGraph Server

Responsabilidades:

-   Escuchar conexiones TCP.
-   Gestionar sesiones.
-   Autenticar usuarios.
-   Autorizar operaciones.
-   Recibir y ejecutar consultas.
-   Enviar resultados y errores.
-   Gestionar transacciones.
-   Limitar recursos.
-   Gestionar conexiones simultáneas.

Ejemplo:

``` bash
mygraph-server start \
    --host 127.0.0.1 \
    --port 7687 \
    --data ./data
```

### 9.2 Protocolo de comunicación

Se desarrollará un protocolo propio, versionado y con framing explícito.

Cabecera conceptual:

``` text
HEADER
├── Protocol Version
├── Message Type
├── Request ID
└── Payload Length

PAYLOAD
├── Authentication
├── Query
├── Parameters
└── Transaction Context
```

Tipos de mensajes:

-   HELLO.
-   AUTH.
-   QUERY.
-   QUERY_RESULT.
-   BEGIN.
-   COMMIT.
-   ROLLBACK.
-   PING.
-   PONG.
-   ERROR.
-   GOODBYE.

El protocolo de transporte será independiente de la gramática Cypher. La
compatibilidad con Bolt podrá evaluarse posteriormente para facilitar el
uso de drivers del ecosistema Neo4j.

### 9.3 MyGraph Client

Conexión:

``` bash
mygraph connect localhost:7687
```

Sesión conceptual:

``` text
MyGraph v0.1.0
Connected to localhost:7687

mygraph> SHOW DATABASES;
mygraph> USE DATABASE ecommerce;
mygraph> MATCH (n) RETURN n LIMIT 10;
mygraph> CREATE (:User {name: "Alice"});
mygraph> PROFILE MATCH (n:User) RETURN n;
mygraph> EXIT;
```

Características:

-   Syntax highlighting.
-   Autocompletado.
-   Historial.
-   Consultas multilínea.
-   Resultados tabulares.
-   Resultados JSON.
-   Errores detallados.
-   Tiempo de ejecución.

### 9.4 Embedded Mode

La API embebida usará el mismo core y no abrirá sockets ni necesitará un
proceso servidor.

Ejemplo conceptual:

``` rust
use mygraph::Database;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::open("./mygraph_data")?;

    let result = db.execute(
        "CREATE (:User {name: 'Alice'})",
        params! {}
    )?;

    println!("{:?}", result);

    Ok(())
}
```

También se ofrecerá:

``` rust
let db = Database::in_memory()?;
```

El core no dependerá de Tokio ni del protocolo de red. Tokio se
utilizará principalmente en el servidor. Las operaciones bloqueantes de
almacenamiento y consultas deberán ejecutarse fuera de los workers
asíncronos.

------------------------------------------------------------------------

## 10. Importación y exportación

### 10.1 Importación Cypher

``` bash
mygraph import \
    --database ecommerce \
    --file backup.cypher \
    --format cypher
```

Requisitos:

-   Procesamiento por lotes.
-   Transacciones configurables.
-   Validación de sintaxis.
-   Reporte de errores por línea.
-   Modo dry-run.
-   Detección de duplicados según restricciones.
-   Política de rollback documentada.

### 10.2 Exportación Cypher

``` bash
mygraph export \
    --database ecommerce \
    --format cypher \
    --output backup.cypher
```

Orden de exportación:

1.  Schemas y metadatos.
2.  Restricciones e índices.
3.  Nodos.
4.  Relaciones y propiedades.

Las relaciones se exportarán después de los nodos, garantizando que sus
extremos existan.

Se definirá un mecanismo para conservar o mapear IDs durante
restauraciones.

### 10.3 Backup binario

Cypher no será el formato principal de backup. Se desarrollará un backup
binario consistente, eficiente y capaz de conservar metadatos e
información necesaria para restauraciones exactas.

------------------------------------------------------------------------

## 11. Seguridad y administración

### Usuarios y roles

Ejemplos conceptuales:

``` cypher
CREATE USER admin
SET PASSWORD 'secure-password';

CREATE ROLE reader;
CREATE ROLE writer;
CREATE ROLE administrator;

GRANT READ ON DATABASE ecommerce TO reader;
GRANT WRITE ON DATABASE ecommerce TO writer;
GRANT ALL ON DATABASE ecommerce TO administrator;
```

Se implementarán:

-   Hash de contraseñas con Argon2id.
-   TLS para conexiones remotas.
-   Autorización por database y schema.
-   Control de acceso administrativo.
-   Límites de tamaño de consultas.
-   Límites de memoria por consulta.
-   Protección frente a recorridos excesivos.
-   Auditoría de operaciones administrativas.

Las contraseñas nunca se almacenarán en texto plano. Los ejemplos de
contraseñas son ilustrativos y no deben utilizarse como credenciales
reales.

------------------------------------------------------------------------

## 12. Arquitectura de crates Rust

Estructura propuesta:

``` text
mygraph/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── README.md
├── LICENSE
│
├── crates/
│   ├── mygraph-core/
│   │   └── src/
│   │       ├── database.rs
│   │       ├── graph.rs
│   │       ├── node.rs
│   │       ├── relationship.rs
│   │       ├── property.rs
│   │       ├── value.rs
│   │       ├── path.rs
│   │       └── lib.rs
│   │
│   ├── mygraph-catalog/
│   │   └── src/
│   │       ├── database_catalog.rs
│   │       ├── schema_catalog.rs
│   │       ├── label_catalog.rs
│   │       └── constraint_catalog.rs
│   │
│   ├── mygraph-cypher/
│   │   └── src/
│   │       ├── lexer/
│   │       ├── parser/
│   │       ├── ast/
│   │       ├── semantic/
│   │       ├── planner/
│   │       ├── optimizer/
│   │       ├── executor/
│   │       └── functions/
│   │
│   ├── mygraph-storage/
│   │   └── src/
│   │       ├── page/
│   │       ├── buffer/
│   │       ├── disk/
│   │       ├── record/
│   │       ├── wal/
│   │       ├── checkpoint/
│   │       └── recovery/
│   │
│   ├── mygraph-index/
│   │   └── src/
│   │       ├── btree/
│   │       ├── label/
│   │       ├── property/
│   │       └── composite/
│   │
│   ├── mygraph-transaction/
│   │   └── src/
│   │       ├── manager.rs
│   │       ├── mvcc.rs
│   │       ├── snapshot.rs
│   │       ├── lock_manager.rs
│   │       └── conflict.rs
│   │
│   ├── mygraph-protocol/
│   │   └── src/
│   │       ├── frame.rs
│   │       ├── message.rs
│   │       └── codec.rs
│   │
│   ├── mygraph-server/
│   │   └── src/
│   │       ├── listener.rs
│   │       ├── connection.rs
│   │       ├── session.rs
│   │       ├── authentication.rs
│   │       ├── authorization.rs
│   │       ├── query_handler.rs
│   │       └── main.rs
│   │
│   ├── mygraph-client/
│   │   └── src/
│   │       ├── connection.rs
│   │       ├── request.rs
│   │       ├── response.rs
│   │       └── lib.rs
│   │
│   ├── mygraph-cli/
│   │   └── src/
│   │       ├── commands/
│   │       ├── repl/
│   │       ├── output/
│   │       └── main.rs
│   │
│   ├── mygraph-import/
│   ├── mygraph-export/
│   ├── mygraph-security/
│   ├── mygraph-metrics/
│   └── mygraph/
│       └── src/lib.rs
│
├── tests/
│   ├── unit/
│   ├── integration/
│   ├── storage/
│   ├── transactions/
│   ├── cypher/
│   ├── compatibility/
│   └── recovery/
│
├── benchmarks/
├── docs/
│   ├── ARCHITECTURE.md
│   ├── STORAGE.md
│   ├── CYPHER.md
│   ├── TRANSACTIONS.md
│   ├── PROTOCOL.md
│   ├── SECURITY.md
│   └── ROADMAP.md
│
└── examples/
    ├── embedded.rs
    └── client.rs
```

### Responsabilidades

  Crate                   Responsabilidad
  ----------------------- -------------------------------------
  `mygraph-core`          Modelo de grafo y API central
  `mygraph-catalog`       Bases de datos, schemas y metadatos
  `mygraph-cypher`        Lenguaje y ejecución de consultas
  `mygraph-storage`       Persistencia, páginas y memoria
  `mygraph-index`         Índices
  `mygraph-transaction`   Transacciones y MVCC
  `mygraph-protocol`      Protocolo de comunicación
  `mygraph-server`        Servidor
  `mygraph-client`        Cliente de conexión
  `mygraph-cli`           Interfaz de terminal
  `mygraph-import`        Importación
  `mygraph-export`        Exportación
  `mygraph-security`      Usuarios, roles y permisos
  `mygraph-metrics`       Métricas
  `mygraph`               API pública embebida

Las dependencias entre crates deberán ser unidireccionales y estar
documentadas para evitar ciclos. Las interfaces entre motor,
almacenamiento y transacciones se definirán antes de paralelizar el
desarrollo.

------------------------------------------------------------------------

## 13. Dependencias Rust recomendadas

  Dependencia            Uso
  ---------------------- --------------------------------
  `tokio`                Runtime asíncrono y networking
  `clap`                 Argumentos de terminal
  `rustyline`            Terminal interactiva
  `tracing`              Logging estructurado
  `tracing-subscriber`   Configuración de logs
  `serde`                Serialización de configuración
  `toml`                 Configuración TOML
  `thiserror`            Errores tipados
  `bytes`                Buffers del protocolo
  `crc32fast`            Checksums
  `argon2`               Hash de contraseñas
  `rustls`               TLS
  `uuid`                 IDs de sesiones y operaciones
  `criterion`            Benchmarks
  `proptest`             Tests basados en propiedades

Para el parser se evaluarán `chumsky` y `winnow` mediante un prototipo
de la gramática.

No se utilizará Bincode como formato persistente definitivo sin una capa
de versionado y compatibilidad. El formato de almacenamiento debe
definir explícitamente su versión y codificación para permitir
migraciones futuras.

------------------------------------------------------------------------

## 14. API pública

Ejemplo conceptual:

``` rust
use mygraph::{Database, Config};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default()
        .storage_path("./mygraph_data")
        .cache_size_mb(512);

    let db = Database::open(config)?;

    let result = db.execute(
        "CREATE (u:User {name: $name, age: $age})",
        params! {
            "name" => "Alice",
            "age" => 25
        }
    )?;

    println!("{:?}", result);

    Ok(())
}
```

Operaciones previstas:

``` rust
let db = Database::in_memory()?;
let db = Database::open("./database")?;

let result = db.execute(
    "MATCH (n) RETURN n",
    params! {}
)?;

let tx = db.begin_transaction()?;
```

La API pública no expondrá detalles internos de páginas, índices ni WAL.
Los tipos y firmas definitivos se fijarán mediante pruebas de API y
documentación.

------------------------------------------------------------------------

## 15. Configuración general

Ejemplo de `config.toml`:

``` toml
[server]
host = "127.0.0.1"
port = 7687
max_connections = 500
tls_enabled = false

[storage]
mode = "hybrid"
data_directory = "./data"
page_size = 16384
cache_size_mb = 1024
wal_enabled = true
sync_on_commit = true
checkpoint_interval_secs = 300

[query]
max_memory_mb = 512
max_execution_time_secs = 60
max_result_rows = 100000
enable_query_cache = true

[transactions]
isolation_level = "snapshot"
max_concurrent_transactions = 1000
timeout_secs = 60

[security]
authentication_enabled = true
authorization_enabled = true

[logging]
level = "info"
format = "json"

[metrics]
enabled = true
```

Los valores son perfiles iniciales de configuración, no límites
definitivos de rendimiento. Se ajustarán mediante benchmarks y pruebas
de carga.

------------------------------------------------------------------------

## 16. Testing y calidad

### 16.1 Tests funcionales

-   Crear nodos.
-   Crear relaciones.
-   Modificar propiedades.
-   Eliminar nodos y relaciones.
-   Crear múltiples schemas.
-   Consultar distintos schemas.
-   Crear y eliminar índices.
-   Ejecutar transacciones.
-   Importar y exportar.

### 16.2 Tests Cypher

Se utilizará el TCK de openCypher como referencia de conformidad.

Cada funcionalidad implementada tendrá:

-   Casos válidos.
-   Casos inválidos.
-   Casos límite.
-   Tests de resultados.
-   Tests de errores.

### 16.3 Tests de almacenamiento

-   Escrituras concurrentes.
-   Lecturas concurrentes.
-   Recuperación después de un crash.
-   Corrupción de WAL.
-   Corrupción de páginas.
-   Checkpoints.
-   Reapertura de bases de datos.
-   Consistencia entre índices y datos.

### 16.4 Tests de rendimiento

  Operación                Métrica
  ------------------------ ---------------------
  CREATE                   Nodos creados/s
  MATCH                    Consultas/s
  Relationship traversal   Tiempo de recorrido
  Index lookup             Latencia
  Import                   Registros/s
  Export                   Registros/s
  Transactions             Transacciones/s
  Concurrent queries       Throughput

Se medirán P50, P95 y P99 de latencia, uso de RAM y rendimiento con
diferentes tamaños de grafos.

Los objetivos de rendimiento absolutos se establecerán cuando exista una
implementación funcional y una máquina de referencia para los
benchmarks.

------------------------------------------------------------------------

## 17. Roadmap de desarrollo

### Fase 1 --- Foundation

-   Workspace Rust.
-   Graph model.
-   Nodes y relationships.
-   Properties.
-   Database y schema catalog.
-   Motor en memoria.
-   API embebida inicial.

**Resultado:** grafo funcional desde Rust.

### Fase 2 --- Cypher MVP

-   Lexer.
-   Parser.
-   AST.
-   Semantic analyzer.
-   CREATE.
-   MATCH.
-   WHERE.
-   RETURN.
-   SET y DELETE.

**Resultado:** consultas reales en Cypher.

### Fase 3 --- Persistent Storage

-   Page Manager.
-   Disk Store.
-   Buffer Pool.
-   WAL.
-   Recovery.
-   Checkpoints.

**Resultado:** persistencia real y recuperación.

### Fase 4 --- Query Engine Advanced

-   MERGE.
-   WITH.
-   Aggregations.
-   ORDER BY.
-   OPTIONAL MATCH.
-   Query Planner.
-   Index Manager.

**Resultado:** motor de consultas avanzado.

### Fase 5 --- Transactions

-   ACID.
-   MVCC.
-   Conflict detection.
-   Concurrent execution.
-   Garbage collection.

**Resultado:** consistencia y concurrencia.

### Fase 6 --- Server & Client

-   TCP Server.
-   Protocol.
-   Authentication.
-   CLI. 
-   Session management.

**Resultado:** MyGraph ejecutándose como servicio.

### Fase 7 --- Import & Export

-   Cypher import.
-   Cypher export.
-   Binary backup.
-   Restore.

**Resultado:** migración y portabilidad.

### Fase 8 --- Production Readiness

-   Security.
-   Metrics.
-   Profiling.
-   Performance optimization.
-   Compatibility TCK.
-   Documentation.

**Resultado:** primera versión estable.

### Fase 9 --- Advanced Features

-   Replication.
-   Clustering.
-   Bolt compatibility.
-   Vector indexes.
-   Full-text search.

**Resultado:** evolución del ecosistema.

------------------------------------------------------------------------

## 18. Criterios de aceptación para MyGraph v1.0

  Área            Criterio
  --------------- ------------------------------------------------
  Graph Engine    CRUD completo de nodos y relaciones
  Schemas         Múltiples schemas aislados
  Databases       Múltiples bases de datos
  Cypher          Conformidad documentada con el perfil elegido
  Storage         Persistencia y recuperación
  Transactions    ACID y concurrencia probada
  Indexes         Índices persistentes y consistentes
  Server          Servicio TCP funcional
  Client          CLI interactivo
  Embedded        Librería Rust funcional
  Import          Importación Cypher
  Export          Exportación Cypher
  Security        Usuarios, roles y permisos
  Testing         Tests unitarios, de integración y recuperación
  Documentation   Documentación técnica y de usuario

La compatibilidad con extensiones de Neo4j y Memgraph se publicará por
separado, indicando qué funciones están implementadas y cuáles no.

------------------------------------------------------------------------

## 19. Comandos oficiales de MyGraph

### Servidor

``` bash
mygraph-server start
mygraph-server stop
mygraph-server status
mygraph-server config
```

### Cliente

``` bash
mygraph connect localhost:7687
mygraph disconnect
mygraph query "MATCH (n) RETURN n"
mygraph shell
```

### Administración

``` bash
mygraph database create ecommerce
mygraph database drop ecommerce
mygraph database list

mygraph schema create ecommerce products
mygraph schema list ecommerce

mygraph user create admin
mygraph backup ecommerce
mygraph restore ecommerce ./backup
```

### Importación y exportación

``` bash
mygraph import ./backup.cypher
mygraph export --format cypher
mygraph export --format binary
```

### Diagnóstico

``` bash
mygraph status
mygraph stats
mygraph explain "MATCH (n) RETURN n"
mygraph profile "MATCH (n) RETURN n"
```

La sintaxis exacta de los comandos se fijará al implementar el CLI y se
mantendrá documentada.

------------------------------------------------------------------------

## 20. Decisiones arquitectónicas fijadas

  -----------------------------------------------------------------------
  Elemento                            Decisión
  ----------------------------------- -----------------------------------
  Lenguaje                            Rust

  Identificadores                     IDs internos de 64 bits, sin
                                      reutilización prematura

  Modelo                              Property Graph dirigido

  Almacenamiento                      Páginas persistentes + caché

  Formato persistente                 Binario propio, versionado

  Recuperación                        WAL + checkpoints

  Concurrencia                        MVCC

  Consultas                           Ejecución mediante operadores

  Protocolo                           TCP propio versionado

  Cliente                             CLI independiente

  Embedded                            API Rust pública

  Seguridad                           Autenticación y RBAC

  Configuración                       TOML

  Observabilidad                      Tracing + métricas

  Compatibilidad                      openCypher como base y perfiles
                                      adicionales
  -----------------------------------------------------------------------

### Restricciones iniciales

1.  Una única instancia de MyGraph tendrá control exclusivo sobre sus
    archivos de almacenamiento.
2.  No habrá clustering ni escritura distribuida en la primera versión.
3.  El core no dependerá de Tokio ni del protocolo de comunicación.
4.  La durabilidad, recuperación y consistencia deberán probarse
    mediante tests de fallos y reinicios.
5.  Las funcionalidades y extensiones compatibles se documentarán por
    versión.

------------------------------------------------------------------------

## 21. Primera versión de desarrollo: MyGraph v0.1

No se recomienda crear todos los crates y dejar decenas de módulos
vacíos. El desarrollo comenzará con una primera versión vertical y
funcional.

### Graph Engine + Cypher básico

Esta primera versión deberá permitir:

-   Crear una base de datos.
-   Crear múltiples schemas.
-   Crear nodos y relaciones.
-   Modificar y eliminar propiedades.
-   Ejecutar MATCH, CREATE, WHERE y RETURN.
-   Ejecutarse desde una API Rust.
-   Guardar y recuperar datos de disco.

### Prueba de funcionamiento

``` cypher
CREATE (a:User {name: "Alice"})
CREATE (b:User {name: "Bob"})
CREATE (a)-[:KNOWS]->(b)

MATCH (u:User)-[:KNOWS]->(f)
RETURN u.name, f.name;
```

**Resultado esperado:** una fila que representa la relación Alice → Bob.

------------------------------------------------------------------------

## 22. Referencias técnicas

-   [openCypher Resources](https://opencypher.org/resources/) ---
    especificación, gramática y TCK.
-   [openCypher GitHub](https://github.com/opencypher) --- repositorios
    y recursos del lenguaje.
-   [Tokio Documentation](https://docs.rs/tokio/latest/tokio/) ---
    runtime asíncrono y networking en Rust.
-   [Tokio Runtime](https://tokio.rs/tokio/topics/runtime) ---
    configuración del runtime.

------------------------------------------------------------------------

## Resumen ejecutivo

**MyGraph** será una base de datos de grafos independiente, desarrollada
en Rust, con:

-   Motor de grafos propio.
-   Almacenamiento híbrido RAM + disco.
-   Formato binario persistente versionado.
-   WAL, checkpoints y recuperación.
-   Transacciones ACID y MVCC.
-   Compatibilidad Cypher como objetivo de conformidad.
-   Múltiples bases de datos y schemas.
-   Índices y restricciones.
-   Servidor TCP y cliente CLI.
-   API Rust embebida.
-   Importación y exportación Cypher.
-   Seguridad, métricas y herramientas de diagnóstico.

**Orden recomendado de implementación:**

Graph Engine → Cypher → Storage → Transactions → Server → Client →
Import/Export → Production.

Este documento es la especificación arquitectónica de referencia. Las
interfaces, formatos binarios, gramática soportada y contratos de
compatibilidad deberán concretarse y versionarse durante la
implementación.
