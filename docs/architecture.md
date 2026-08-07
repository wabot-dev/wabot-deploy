# wabot-deploy — Arquitectura

Estado: propuesta acordada. Alcance: la **Fase 1 — autoconfiguración del
nodo**. Las fases posteriores (registry OCI, despliegues, consola) se
esbozan solo lo suficiente para no cerrarnos puertas ni construir dos veces.

Premisa que ordena el documento: **el framework es nuestro**. En cada
decisión la pregunta no es "¿cómo esquivo esta limitación?" sino "¿esto es
una capacidad genérica que le falta a wabot-rust, o es producto de
wabot-deploy?". Cuando es lo primero va al framework y lo hereda cualquier
otra app; cuando es lo segundo se queda aquí, aunque duela escribirlo.

Decisiones tomadas: el framework aporta **solo la base TLS**, no el edge
completo · runtime **crun**, no runc · arranque **sin dominio** con
autofirmado · **framework primero**, luego producto.

---

## 1. Qué es

`wabot-deploy` es un **binario único** que se instala en un nodo Linux y lo
convierte en una plataforma de despliegue de contenedores equivalente a
wabot-cloud + wabot-console, mononodo y sobre infraestructura propia.

| wabot-cloud | wabot-deploy |
| --- | --- |
| Kubernetes multi-nodo (UpCloud) | containerd + crun, un nodo |
| Harbor | registry OCI embebido sobre el content store de containerd |
| ingress-nginx + cert-manager | edge embebido: TLS, routing por host, ACME |
| Postgres | SQLite |
| Node.js / TypeScript | Rust / wabot-rust |
| N procesos y pods de sistema | 2 procesos: `containerd` + `wabot-deploy` |

Objetivo transversal: **RAM mínima**. Presupuesto `wabot-deploy` < 40 MB RSS,
`containerd` ~40–60 MB, plano de control total < 100 MB. Por cada contenedor
del usuario hay además un shim de ~11 MB que no depende de nosotros (§7.3).

---

## 2. El reparto: framework vs producto

### 2.1 Va al framework — cuatro cambios

Ninguno es específico de containerd ni de despliegues. Los cuatro le faltan
hoy a cualquier app wabot-rust que quiera correr self-hosted en una VM.

| # | Cambio | Crate | Por qué es framework |
| --- | --- | --- | --- |
| **F1** | Cuerpos grandes y respuestas crudas en controllers | `wabot-macros`, `wabot-feature-rest-controller` | Hoy es un muro: 1 MiB fijo y siempre `(200, Json)`. Bloquea subidas, descargas, SSE y streaming en general |
| **F2** | TLS y apagado grácil en el servidor REST | `wabot-feature-rest-controller`, feature `tls` | El propio `server.rs:13` dice *"Extend here as real apps need TLS"*. Ninguna app puede servir HTTPS hoy |
| **F3** | Backend SQLite con paridad de Postgres | `wabot-feature-sqlite` + `wabot-addon-async-sqlite` (nuevos) | `pg` es el único backend. Sin esto no hay despliegue embebido, ni tests sin una base de datos externa |
| **F5** | Cancelación cooperativa y `sd_notify` | `wabot-core` (lifecycle), `wabot` (runner) | El `ProjectRunner` no entrega señal de cancelación a los servicios: la fase de *drain* que documenta CLAUDE.md hoy no tiene a quién esperar |

*(Se conserva la numeración: lo que era F4 —el edge— se queda en la app.)*

### 2.2 Se queda en wabot-deploy

- **El edge completo**: routing por header `Host`, proxy inverso con
  upgrades, y ACME. El framework solo pone el listener TLS (F2).
- Cliente de containerd, generación de specs OCI, selección de crun
- El registry `/v2` y el puente con el content store
- El bootstrap del nodo: preflight, instalación de containerd + crun, unit de
  systemd, ledger de pasos
- El modelo de dominio: `project`, `service`, `deployment`, `config_group`
- La consola

**Por qué el edge no sube al framework** (decisión revisada): el listener TLS
con resolver de certificados dinámico es genérico y sube como F2; el resto
—qué host va a qué contenedor, cuándo pedir un certificado, cómo se
persisten las rutas— está acoplado al modelo de despliegue. Un
`wabot-feature-edge` con traits `RouteTable`/`CertStore` sonaba limpio, pero
sería una abstracción diseñada con un solo consumidor. Si aparece un
segundo, se extrae entonces, con dos usos reales encima de la mesa.

### 2.3 Lo que explícitamente NO hacemos en el framework

- **No un `wabot-feature-containerd`.** Es el producto.
- **No un segundo stack HTTP.** El edge usa `hyper::server::conn::auto`
  llamando a un `axum::Router` como `tower::Service` — que es exactamente lo
  que hace `axum::serve` por dentro. La convención de CLAUDE.md ("HTTP work
  goes through axum") se respeta.
- **No middleware que decore la respuesta.** Sigue siendo rejection-only; lo
  que haya que decorar va en un layer de tower, como ya dice su Fase 2.
- **No romper el contrato existente.** F1 y F2 son aditivos: un
  `#[rest_controller]` escrito hoy compila igual mañana, y lo nuevo es opt-in
  detrás de features.

---

## 3. Los cambios de framework, en concreto

### F1 — cuerpos grandes y respuestas crudas

Dos cambios independientes en `crates/wabot-macros/src/rest.rs` y
`crates/wabot-feature-rest-controller/src/runtime.rs`.

**F1a — `#[max_body(N)]` por endpoint.** La fontanería ya existe:
`decompose_request(request, max_body)` toma el límite como parámetro
(`runtime.rs:36`) y el macro le pasa la constante `DEFAULT_MAX_BODY`
(`rest.rs:302`). Hace falta un campo más en `struct Endpoint`
(`rest.rs:129`), una rama más en el bucle de atributos (`rest.rs:176`), y
sustituir la constante por la expresión. Es un cambio de una tarde.

```rust
#[post("/upload")]
#[max_body(64 * 1024 * 1024)]
async fn upload(&self, req: Upload) -> RestResult<Receipt> { … }
```

**F1b — endpoints crudos.** Para lo que no cabe en memoria y lo que no es
JSON:

```rust
#[get("/blobs/:digest", raw)]
async fn get_blob(&self, req: Request) -> RestResult<Response> { … }
```

Con `raw`, el macro emite un wrapper que:

1. hace `request.into_parts()`,
2. corre los middlewares sobre `&parts` — se conservan, y por eso los guards
   siguen funcionando en estas rutas,
3. reensambla con `Request::from_parts(parts, body)` y se lo pasa al handler,
4. devuelve el `Response` tal cual, sin pasar por `ok_json`.

Esto desbloquea los blobs OCI (push y pull en streaming), SSE para logs de
contenedor en vivo, y descargas de ficheros. Sin ello esas rutas se escriben
a mano fuera del sistema de controllers y pierden DI, middlewares y el
harness de tests — coste que no queremos pagar en un producto entero.

`lib.rs:35` ya documenta `RestResult<axum::response::Response>` en un ejemplo
que hoy **no compila**. F1b hace verdad la documentación.

### F2 — TLS y apagado grácil

`RestServerConfig` pasa de un campo a cuatro, todo aditivo:

```rust
pub struct RestServerConfig {
    pub bind_addr: SocketAddr,
    pub tls: Option<TlsMode>,                 // feature "tls"
    pub shutdown: Option<CancellationToken>,
    pub drain_timeout: Duration,
}

pub enum TlsMode {
    Static(Arc<rustls::ServerConfig>),
    Resolver(Arc<dyn rustls::server::ResolvesServerCert>),   // SNI dinámico
}
```

Más `serve_on(listener, router, config)` para la app que traiga su propio
listener — socket activado por systemd, o el nuestro del edge.

`TlsMode::Resolver` es la pieza que importa: es lo que permite que
wabot-deploy enchufe su propio resolver con certificados que aparecen y se
renuevan en caliente, sin que el framework sepa nada de ACME.

`rustls` entra detrás de una feature `tls`, apagada por defecto — la misma
disciplina que el crate ya aplica a teloxide, socketioxide y sqlx.

El apagado grácil es la otra mitad: hoy `ProjectRunner` suelta el future del
servicio y las peticiones en vuelo se cortan. Con `shutdown` +
`drain_timeout`, `axum::serve(...).with_graceful_shutdown(...)`.

### F3 — SQLite

`wabot-feature-sqlite` espeja `wabot-feature-pg` módulo a módulo:

| pg | sqlite | notas |
| --- | --- | --- |
| `jsonb.rs` → `PgJsonbStore` | `SqliteJsonbStore` | `json_extract` / `->>` (SQLite ≥ 3.38) en lugar de operadores jsonb; columnas **generadas** en vez de promovidas |
| `columns.rs` → `PgColumnsStore` | `SqliteColumnsStore` | igual |
| `query_sql.rs` → `build_query_sql` | ídem, grafías SQLite | ver abajo |
| `locker.rs` → `PgLocker` | — | innecesario: mononodo, `InProcessLocker` es correcto |
| `migrations/` + `wabot-migrate` | mismo runner sobre `PRAGMA user_version` | el binario aprende `--sqlite` |
| `transaction.rs`, `projection.rs`, `infra.rs` | ídem | |

Y `wabot-addon-async-sqlite` con `JobRepository` / `CronJobRepository`,
calcando `wabot-addon-async-in-memory` (279 LOC). Esto **es requisito**: los
jobs de despliegue tienen que sobrevivir a un reinicio del nodo, así que el
repo en memoria no sirve.

Driver: `rusqlite` con `bundled`. **No `sqlx-sqlite`**: levanta un hilo del
SO por conexión del pool y habla con él por canales — exactamente el coste
que estamos evitando — y su verificación de queries en compilación exige
`DATABASE_URL` o caché offline en el build. `bundled` además compila SQLite
dentro del binario, condición para "un solo binario".

Concurrencia: **un hilo escritor dedicado con canal** + un par de conexiones
de lectura. SQLite admite un escritor; un pool ingenuo produce `SQLITE_BUSY`
en cascada. Pragmas: `journal_mode=WAL`, `synchronous=NORMAL`,
`busy_timeout=5000`, `foreign_keys=ON`, `cache_size=-2000`. **Sin
`mmap_size`** — infla el RSS sin beneficio a nuestros tamaños.

**Sobre el dialecto SQL:** la tentación es extraer ya un `SqlDialect` a
`wabot-core` compartido por pg y sqlite. Propongo **no hacerlo todavía**:
`build_query_sql` se escribe primero con grafías SQLite en el crate nuevo, y
la extracción se hace **al cerrar F3**, cuando existan los dos y el diff sea
visible. Abstraer un dialecto antes de tener dos implementaciones es cómo se
acierta al 60% y luego se pelea con la abstracción. Va como tarea de cierre,
no como opcional — si se deja abierta, los dos ficheros divergen.

### F5 — cancelación y `sd_notify`

```rust
runner.service_with_cancel("edge", |token| edge::serve(state, token))
```

El `ShutdownManager` ya orquesta intake → drain → close; lo que falta es
entregarle al servicio el token para que **pueda** drenar.

Y `sd_notify`: `READY=1` cuando los servicios están arriba, `STOPPING=1` al
entrar en shutdown, `WATCHDOG=1` periódico si la unit lo pide. Son ~40 LOC
sin dependencias (un `SOCK_DGRAM` a `$NOTIFY_SOCKET`) y habilitan
`Type=notify`, que es lo que hace que `systemctl start` no vuelva hasta que
el nodo sirve de verdad. El paso 9 del bootstrap depende de eso.

---

## 4. wabot-deploy

```
src/
  main.rs           CLI: install | serve | doctor
  config.rs         config.toml + override por env
  bootstrap/        LA FASE 1
    preflight.rs    checks del nodo
    layout.rs       directorios y permisos
    containerd.rs   detectar / instalar / configurar containerd + crun
    service.rs      unit de systemd
    ledger.rs       node_state — pasos idempotentes
  edge/             LA OTRA MITAD DE LA FASE 1
    server.rs       :443 sobre RestServerConfig::tls(Resolver) — F2
    resolver.rs     impl ResolvesServerCert sobre la tabla certificate
    routes.rs       Host -> Upstream, ArcSwap
    proxy.rs        forward a contenedor local + hyper::upgrade
    http.rs         :80 — reto ACME y 301
    acme/           instant-acme: cuenta, órdenes, bucle de renovación
  api/              #[rest_controller] del plano de control
  console/          #[ui_controller] Maud
  registry/         FASE 2 — /v2, endpoints #[raw]
  runtime/          FASE 2 — cliente containerd, specs OCI, crun
  domain/           FASE 3 — project, service, deployment
```

Features del framework:

```toml
wabot = { version = "0.1", default-features = false, features = [
    "rest", "rest-tls",     # F2
    "sqlite",               # F3
    "async-jobs", "addon-async-sqlite",
    "ui",                   # Maud: sin build step de Node, sin payload de cliente
    "tracing-format",
] }
```

`default-features = false` es obligatorio: el default arrastra `chat`,
`chatbot` y `addon-cmd`, es decir todo el stack LLM.

### El edge

```
                    :80                          :443 (rustls, vía F2)
                     │                                │
          ┌──────────┴──────────┐          ┌──────────┴───────────┐
          │ ACME http-01        │          │ SNI → nuestro resolver
          │ resto → 301 https   │          │      (tabla certificate)
          └─────────────────────┘          └──────────┬───────────┘
                                            despacho por header Host
                    ┌─────────────────┬───────────────┼────────────────┐
              consola + API      registry /v2      app del user     404
              (controllers,      (endpoints raw,   (proxy hyper
               in-process)        F1b)              → 127.0.0.1:p)
```

Detalles que importan y que son fáciles de equivocar:

- **Despacho por `Host`, no por SNI.** Con HTTP/2 una conexión TLS puede
  transportar peticiones de varios hostnames (*connection coalescing*). El
  SNI solo elige certificado; el routing se decide por petición.
- **La consola y el API no salen a la red.** El `Router` es un
  `tower::Service` y se invoca con `oneshot`. Cero sockets.
- **Tabla de rutas en `ArcSwap`**, hidratada al arrancar y reemplazada entera
  en cada cambio. Lecturas sin lock, escrituras raras.
- **`hyper::upgrade::on()` en ambas patas** del proxy. WebSockets son
  requisito, y es la parte que más se rompe en proxies escritos a mano.

### Certificados

`instant-acme`. **No `rustls-acme`**: su lista de dominios se fija al
construir `AcmeConfig` y no hay API para añadir uno en caliente — y nuestra
plataforma es dinámica por definición.

**Regla dura: `ResolvesServerCert::resolve()` es síncrono.** No se emite nada
dentro del handshake. Se emite **al enlazar el hostname**, que es cuando lo
conocemos, y se renueva en background. Esto además cierra el vector de DoS de
la emisión on-demand: quien mande SNIs aleatorios no puede quemarnos el rate
limit de Let's Encrypt (50 certs por dominio registrado y semana).

Retos: **HTTP-01 primero** — necesitamos :80 abierto igual, para el redirect.
TLS-ALPN-01 después como fallback cuando :80 no esté disponible. DNS-01 solo
si hacemos wildcards, y ahí hay que escribir el cliente de cada proveedor a
mano: no hay equivalente maduro a `lego` en Rust.

### Esquema SQLite de Fase 1

```sql
node_state    (step TEXT PK, status, detail JSON, updated_at)
setting       (key TEXT PK, value TEXT)
acme_account  (directory_url TEXT PK, email, key_pem, kid, created_at)
certificate   (domain TEXT PK, cert_pem, key_pem, issuer,
               issued_at, not_after, status, last_error)
route         (host TEXT PK, upstream_kind, upstream_addr, service_id, enabled)
```

Las fases siguientes añaden `project`, `service`, `deployment`,
`config_group`, `image` sobre `SqliteJsonbStore`, con layout
`(id, created_at, updated_at, data JSON)` — el mismo que usa el Postgres de
wabot-cloud. Un solo modelo mental entre las dos plataformas, y la puerta
abierta a compartir el cliente de API de la consola.

---

## 5. Autoconfiguración del nodo — la Fase 1

`wabot-deploy install` es una secuencia de pasos idempotentes. Cada paso se
registra en `node_state` antes y después, así que volver a correr `install`
converge en lugar de duplicar, y un fallo a mitad se reanuda donde quedó.
`--force <step>` rehace uno concreto.

| # | Paso | Qué hace | Idempotencia |
| --- | --- | --- | --- |
| 1 | `preflight` | Linux, root, arch x86_64/aarch64, systemd, **cgroup v2 unified**, overlayfs, :80 y :443 libres, disco | read-only |
| 2 | `layout` | `/etc/wabot-deploy/`, `/var/lib/wabot-deploy/{db,certs}`, modo 0700 root | `mkdir -p` |
| 3 | `config` | Escribe `config.toml` si no existe; genera token de admin | no sobreescribe |
| 4 | `database` | Crea la BD y aplica migraciones | `PRAGMA user_version` |
| 5 | `runtime` | Instala/valida **containerd** y **crun**; escribe `/etc/containerd/config.toml`; `enable --now`; espera al socket | detecta antes de instalar; no pisa config ajena |
| 6 | `binary` | Copia `/proc/self/exe` a `/usr/local/bin/` si difiere | compara hash |
| 7 | `service` | Escribe la unit, `daemon-reload`, `enable` | la unit es nuestra |
| 8 | `certificate` | Con dominio → orden ACME **síncrona**, para que el fallo de DNS se vea ahí mismo. Sin dominio → CA local + hoja autofirmada | reusa si válido > 30 días |
| 9 | `start` | `systemctl start` (espera `READY=1` vía F5) + health-poll a `https://127.0.0.1/healthz` | `restart` si ya corría |
| 10 | `report` | URL, token de admin, fingerprint si es autofirmado | — |

### Arranque sin dominio

Si no hay `node.domain`, generamos CA local + hoja autofirmada (`rcgen`) para
`https://<ip>` y `https://localhost`, e imprimimos el fingerprint al final.
Permite probar el nodo antes de tener DNS y hace posible el CI en VM
efímera. ACME se activa después con `install --domain ...`, sin reinstalar
nada: el paso 8 vuelve a correr y el resto converge sin cambios.

### containerd + crun

**crun en lugar de runc.** Es la decisión correcta para este producto: crun
es C sin garbage collector, **300 KB de binario frente a los 15 MB de runc**,
y arranca contenedores un 15–25 % más rápido. Ver §7.3 para qué ahorra
exactamente y qué no.

**La buena noticia: con el API nativo de containerd, elegir crun no es
configuración global.** El runtime se selecciona **por contenedor**, en
`Containers.Create`:

```
Container.runtime = Runtime {
    name:    "io.containerd.runc.v2",       // el shim, no el runtime
    options: Any(containerd.runc.v1.Options {
        BinaryName:    "/usr/local/bin/crun",
        SystemdCgroup: true,
    }),
}
```

El shim `io.containerd.runc.v2` está mal llamado: no es "el shim de runc",
es el shim para runtimes compatibles con la CLI de runc, y `BinaryName`
elige cuál. crun lo es. Consecuencia práctica: el paso 5 **no** necesita
editar `/etc/containerd/config.toml` para esto, y un containerd preexistente
—el que instaló Docker, por ejemplo— nos sirve sin tocarlo.

**El coste: `containerd-client` no incluye ese proto.**
`containerd.runc.v1.Options` vive en `runtime/v2/runc/options/oci.proto`, que
el crate no vendoriza (solo trae `services`, `types`, `events`, `google`).
Hay que generarlo nosotros con prost — es **un mensaje de ~11 campos**, así
que es media hora de build script, pero conviene saberlo antes de empezar y
no descubrirlo a mitad de la Fase 2.

**Método de instalación: tarball estático oficial**, y la decisión de crun
lo refuerza. El argumento: el paquete de distro te da versión y config por
defecto distintas en cada release de Ubuntu/Debian/RHEL, y esa divergencia se
paga en soporte. Pero además ya íbamos a bajar un binario fuera del gestor de
paquetes de todos modos — el tarball de containerd **no incluye** runtime, y
lo que traen los paquetes de distro es runc, no crun. Así que:

- `containerd-<ver>-linux-<arch>.tar.gz` desde GitHub releases, con checksum
- `crun-<ver>-linux-<arch>` (estático) desde `containers/crun` releases, con checksum
- versiones fijadas por nosotros, las mismas que probamos en CI

Si ya hay un containerd corriendo, **no lo tocamos**: detectamos, verificamos
versión mínima y usamos su socket. crun sí lo instalamos igual si falta,
porque es nuestro y no interfiere con nada.

Lo que sí escribimos en `/etc/containerd/config.toml`: snapshotter
`overlayfs`, y el host de registry en HTTP plano para `127.0.0.1` (lo pide la
Fase 2, §7.1). `SystemdCgroup` va en las opciones por contenedor, no aquí —
pero es igual de necesario: sin él los límites de memoria y la contabilidad
de OOM no funcionan bien, y el autosizing que heredamos de wabot-cloud
depende de leer OOM correctamente.

### Unit de systemd

```ini
[Unit]
Description=wabot-deploy
After=network-online.target containerd.service
Wants=network-online.target
Requires=containerd.service

[Service]
Type=notify
ExecStart=/usr/local/bin/wabot-deploy serve
Restart=always
RestartSec=2
LimitNOFILE=65535
NoNewPrivileges=yes
ProtectHome=yes
```

Corre como **root**: necesita el socket de containerd, escribir en
`/var/lib/wabot-deploy` y enlazar :443. Se puede endurecer más adelante con
`CAP_NET_BIND_SERVICE` y un usuario dedicado, pero no en la primera versión —
el hardening prematuro aquí produce fallos silenciosos difíciles de
diagnosticar.

### Composición de `serve`

```rust
ProjectRunner::new(container.clone())
    .service_with_cancel("edge",  |t| edge::serve(edge_cfg, t))      // F2 + F5
    .service_with_cancel("acme",  |t| edge::acme::renewal_loop(db, t))
    .service("jobs", run_async_workers(container.clone(), commands, crons))
    .on_shutdown(ShutdownTask::new("db", ShutdownPhase::Close, checkpoint_wal))
    .run()
    .await
```

El primer servicio que termina tumba el proceso — que es lo correcto: un nodo
cuyo :443 murió no debe seguir pareciendo sano.

**Distros de Fase 1:** Debian 12+ y Ubuntu 22.04/24.04 como soporte oficial;
el preflight avisa y sigue en best-effort en el resto. RHEL y derivados
cuando toque, y el trabajo real allí es SELinux en *enforcing* (etiquetado
del content store y de los bundles), no la instalación.

---

## 6. Dependencias

```toml
# framework
wabot = { version = "0.1", default-features = false, features = [...] }

# nuevas dentro del framework
rustls        = "0.23"                              # rest-controller, feature tls
tokio-rustls  = "0.26"
rusqlite      = { version = "0.40", features = ["bundled", "serde_json"] }

# wabot-deploy
hyper         = { version = "1", features = ["server", "client", "http1", "http2"] }
hyper-util    = "0.1"
instant-acme  = "0.8"
rcgen         = "0.14"      # CSRs + autofirmado del arranque sin dominio
arc-swap      = "1"
toml          = "0.8"
containerd-client = "0.9"                                    # Fase 2
oci-spec          = { version = "0.10", features = ["runtime", "image"] }
prost / prost-build                                          # el proto de crun (§5)
```

`tokio` con features explícitas, no `"full"`. Y `worker_threads` acotado
(default 2): el default de tokio es un worker por core, y en un nodo de 16
cores son 16 stacks para un plano de control que casi no hace CPU.

Nota sobre `containerd-client`: son bindings generados, sin cliente de alto
nivel — no hay `Pull`, ni `NewContainer`, ni generación de spec. Esa capa la
escribimos nosotros. Sus protos van ~2 minor por detrás de containerd
estable, irrelevante para content/images/snapshots/tasks, que llevan años
estables. Requiere `protoc` en la máquina de build.

---

## 7. Fases siguientes — lo que ya condiciona el diseño

### 7.1 Registry OCI compartido con containerd

Funciona y es un uso legítimo del API: los blobs se escriben *a través* del
servicio Content (`Write` con `action=WRITE/COMMIT`, que mapea casi 1:1 sobre
el upload chunked de la Distribution Spec), nunca directamente en
`blobs/sha256/`. Cuatro cosas críticas:

1. **Registry y workloads en el mismo namespace de containerd.** Si no, el
   contenido no es visible entre namespaces y se pierde todo el ahorro.
2. **Lease sostenido durante todo el push.** Entre el `COMMIT` del último
   blob y el `Images.Create` del manifest los blobs no tienen referencias, y
   un pase de GC se los lleva. El `Images.Create` es luego el GC root.
3. **Labels de GC con las convenciones de containerd**
   (`containerd.io/gc.ref.content.config`, `.l.<i>`, `.m.<i>`,
   `containerd.io/uncompressed`). Inventar las nuestras rompe `ctr`,
   `nerdctl` y el propio GC.
4. Escribir por gRPC, **leer del filesystem**. Servir un `GET blobs/` a
   través del stream de protobuf sobre unix socket es caro, y los blobs son
   inmutables y direccionables por contenido.

El punto no resuelto: un `docker push` a nuestro registry deja los blobs pero
**no** el snapshot, así que no se puede arrancar una task. La salida más
barata es que containerd se haga un pull a sí mismo por loopback vía el
servicio Transfer — los blobs ya existen, así que no descarga nada, pero
obtenemos el unpack correcto (chain IDs, labels) gratis. Por eso el paso 5
configura el registry en HTTP plano para localhost.

### 7.2 Ejecución de contenedores

Tasks de containerd directamente, no systemd. containerd ya supervisa vía el
shim y reporta salidas por el servicio Events; meter systemd crea dos fuentes
de verdad sobre "¿esto está corriendo?". El shim además sobrevive a
reinicios de containerd, así que actualizar nuestro binario no mata las
cargas del usuario.

Specs OCI con `oci-spec` (`runtime` + `image`), no `serde_json::json!` a
mano. Ojo: `Spec::default()` no equivale a lo que genera containerd
(`oci.WithDefaultSpec` + `WithDefaultUnixDevices` + `WithImageConfig`) —
faltan los mounts estándar de `/proc`, `/dev`, `/sys`, las reglas de device
cgroup, el juego de namespaces, los masked/readonly paths, y el merge de la
config de la imagen. Son unos cientos de líneas y es la parte más tediosa del
proyecto.

### 7.3 Dónde está de verdad la RAM por contenedor

Conviene tener el número honesto antes de optimizar lo que no toca.

| Componente | Coste | ¿Lo cambia crun? |
| --- | --- | --- |
| `containerd-shim-runc-v2`, uno por contenedor | **~11 MB RSS, persistente** | **No** |
| El proceso del runtime (`crun`/`runc`) | efímero: crea el contenedor y sale | Sí — 300 KB vs 15 MB, y 15–25 % menos latencia de arranque |
| El proceso del usuario | lo que sea | No |

Es decir: **crun gana en tamaño de binario, en latencia de arranque y en la
memoria pico de cada `create`/`exec`, no en el residente por contenedor.** El
suelo persistente es el shim, y es Go.

Si algún día ese suelo estorba, existe una palanca documentada: el shim
`runc.v2` **agrupa contenedores en un solo proceso** según labels — es lo que
hace CRI con `io.kubernetes.cri.sandbox-id` para meter todos los contenedores
de un pod en un shim. Podríamos agrupar por servicio. No hace falta en Fase 1
(mononodo, una réplica por servicio ⇒ la relación ya es 1:1), pero es la
respuesta cuando aparezcan sidecars o servicios multi-contenedor. Verificar
sobre nuestra versión de containerd antes de depender de ello: es una
propiedad de la implementación del shim, no del API.

### 7.4 Logs en vivo

SSE sobre un endpoint `#[raw]` (F1b), no socket.io. wabot-cloud usa un
namespace de socket.io porque ya tenía el servidor montado; aquí SSE es
unidireccional, cabe en el edge sin nada extra, y ahorra el árbol de
dependencias de socketioxide.

---

## 8. Plan de entrega

El framework va delante, en trozos pequeños, en orden de bloqueo. Cada uno es
mergeable y útil por separado.

| Hito | Dónde | Contenido | Verificación |
| --- | --- | --- | --- |
| **A1** ✅ | framework | **F5** cancelación + `sd_notify`; **F2** TLS y apagado grácil | hecho — ver §8.1 |
| **A2** ✅ | framework | **F3** `wabot-feature-sqlite` + `wabot-addon-async-sqlite` | hecho — ver §8.2 |
| **A3** ✅ | framework | **F1** `#[max_body]` y endpoints `#[raw]` | hecho — ver §8.3 |
| **M0** ✅ | deploy | CLI, config, BD, `ProjectRunner`, `/healthz` | hecho — ver §8.4 |
| **M1** ✅ | deploy | Edge completo | hecho — ver §8.5 |
| **M2** ✅ | deploy | ACME real, bucle de renovación | hecho — ver §8.6 |
| **M3** | deploy | Bootstrap completo: preflight, containerd + crun, unit, ledger | VM limpia Ubuntu 24.04 |
| **M4** | deploy | Endurecimiento | idempotencia, reanudación tras fallo, RSS |

**El framework ya no bloquea nada**: A1, A2 y A3 cerrados. Todo lo que
queda es producto.

### 8.1 A1 — hecho

En `wabot-rust`, sin romper nada existente (300 tests verdes entre los crates
tocados y sus dependientes; `cargo check --workspace --all-features` limpio).

**F5 — cancelación cooperativa**

- `wabot_core::lifecycle::Cancel` (nuevo, `core/lifecycle/cancel.rs`). Canal
  `watch`, no un flag: un flag hay que sondearlo y un listener parado en
  `accept()` nunca llega a sondearlo. **Latched, no un flanco** —
  `cancelled()` sobre una señal ya disparada vuelve inmediatamente, para que
  un servicio que arranca durante un apagado en curso no espere una segunda
  cancelación que no llegará.
- `ShutdownManager::cancel_signal()` / `cancel()` / `timeout()`. La señal se
  dispara **antes de la primera fase**: un servicio que sigue aceptando
  mientras corren las tareas de intake le sigue dando trabajo al drain.
- `ProjectRunner::service_with_cancel(name, |cancel| ...)`.
- **El runner ahora espera a sus servicios.** Antes, la rama de señal del
  `select!` soltaba el `select_all` entero y con él todos los futures — que
  era el bug de fondo. Ahora van a un `JoinSet`, se dispara el cancel, se
  drena bajo `ShutdownManager::timeout()`, y solo se aborta lo que quede en
  el plazo, nombrándolo en el log.
- El closure se resuelve en `run()`, no al registrar. Al principio lo hice al
  registrar y eso creaba una regla de orden invisible: un `with_shutdown()`
  posterior dejaba el servicio atado a un manager descartado. Un test fija
  ese orden.

**F5 — systemd**

- `wabot_core::lifecycle::systemd`: `notify_ready`, `notify_stopping`,
  `notify_status`, `notify_watchdog`, `watchdog_interval`, `is_available`.
  ~100 líneas, sin enlazar libsystemd. No-op cuando no hay `NOTIFY_SOCKET`.
- El runner manda `READY=1` / `STOPPING=1` y lleva el pinger del watchdog,
  así que una app nunca escribe un `cfg`. `WATCHDOG_PID` se respeta y el
  intervalo es **la mitad** de `WATCHDOG_USEC`.
- Verificado compilando contra `aarch64-unknown-linux-gnu`, no solo en
  darwin — la rama que habla con el socket es Linux-only y en macOS no se
  compila siquiera.

**F2 — TLS y apagado grácil**

- `RestServerConfig` gana `with_shutdown`, `with_drain_timeout`, `with_tls`,
  `with_tls_resolver`. Los campos opcionales son privados con builders, así
  que encender o apagar la feature `tls` nunca cambia el código que lo
  construye.
- `serve_on(listener, router, config)` para un listener ya enlazado —
  socket activado por systemd, o puerto 0 en tests.
- `TlsMode::{Static, Resolver}`. El que importa es `Resolver`: se consulta en
  cada handshake, que es lo que permite que los certificados aparezcan y se
  renueven en caliente. Feature `rest-tls` en el paraguas, apagada por
  defecto.
- ALPN (`h2`, `http/1.1`) lo pone el framework: olvidarlo es invisible —
  todo funciona, simplemente todos los clientes negocian HTTP/1.1.
- `serve_connection_with_upgrades`, no `serve_connection`. Sin eso todo
  WebSocket sobre TLS falla, y *solo* sobre TLS.
- El handshake corre en la task de la conexión, nunca en el accept loop. Hay
  un test que manda texto plano al puerto TLS y comprueba que el siguiente
  cliente real sigue siendo atendido.
- `ring` en vez del `aws-lc-rs` por defecto: sin cmake ni nasm.

### 8.2 A2 — hecho

`wabot-feature-sqlite` + `wabot-addon-async-sqlite` en el framework. 51
tests nuevos; **564 tests verdes en los 35 crates** del workspace y
`cargo check --workspace --all-features` limpio.

**Lo que hay**

- `SqliteDatabase` / `SqliteConfig` — un escritor tras un mutex y una
  free-list de lectores, WAL, y las pragmas del plan.
- `SqliteJsonbStore` — mismo layout que el Postgres
  (`id` / `created_at` / `data` + columnas promovidas), tabla creada al
  primer uso, implementa `CrudRepository`.
- `MigrationRunner` con checksums y detección de drift.
- `SqliteJobRepository` / `SqliteCronJobRepository` — los jobs de
  despliegue ya sobreviven a un reinicio.
- Features `sqlite` y `addon-async-sqlite` en el paraguas.

**Tres cosas que salieron distintas de lo previsto**

1. **`created_at` es INTEGER de epoch-millis**, no un timestamp. SQLite
   no tiene tipo fecha; una cadena ISO ordena bien pero cuesta más por
   fila, y la paginación keyset compara esa columna constantemente.

2. **El `PRAGMA case_sensitive_like=ON` no es cosmético.** El `LIKE` de
   SQLite es *insensible* a mayúsculas por defecto y sensible en todo lo
   demás donde corre el framework, incluido el evaluador en memoria.
   Sin la pragma, el mismo `Query` significaría cosas distintas según el
   backend — que es justo el fallo que un AST agnóstico existe para
   evitar. Hay un test que lo fija.

3. **Donde SQLite gana:** su `->>` preserva tipos, así que un filtro de
   rango sobre un campo del documento compara numéricamente sin ningún
   cast. Postgres necesita `::numeric` o `"9" > "10"` sale verdadero.
   Lo que sí hubo que hacer a mano es expandir `In` a un placeholder por
   elemento (no existe `= ANY`), y ahí `bind_values` tiene que aplanar
   el array o todos los valores posteriores caen en la condición
   equivocada.

**Un hallazgo colateral que valía el desvío.** `libsqlite3-sys` declara
`links = "sqlite3"`, y cargo solo admite una versión de un crate con
`links` — regla que aplica en la resolución de *versiones*, antes que
las features. Al añadir rusqlite saltó el conflicto, y al tirar del hilo:
la feature `macros` de sqlx arrastra `sqlx-macros-core`, que depende de
`sqlx-sqlite`. **Todas las builds solo-Postgres venían compilando un
driver de SQLite.** Nadie usa `query!`/`query_as!` en el workspace — el
propio comentario del manifiesto ya lo decía. Quitada la feature, y
`rusqlite` queda fijado a 0.32 para casar con el `libsqlite3-sys 0.30`
de sqlx 0.8. No se pierde nada: 0.30 empaqueta SQLite 3.46, muy por
encima del 3.38 que introdujo `->>`.

**Lo que no está** (y no bloquea M0): `SqliteColumnsStore`, projections,
y la extracción del `SqlDialect` compartido con Postgres. Esto último
ahora sí procede — ya hay dos implementaciones de las que derivarlo, y
las diferencias reales son visibles y pocas. Queda anotado al final de
`query_sql.rs` para hacerlo antes de que aparezca un tercer backend.

**Una corrección al plan:** el camino sin TLS sigue usando `axum::serve` +
`with_graceful_shutdown`. Solo el camino TLS conduce
`hyper_util::server::conn::auto` a mano, porque `axum::serve` de axum 0.7
enlaza un `TcpListener` pelado y no acepta un acceptor. El caso común se
queda en la vía trillada.

### 8.3 A3 — hecho

`#[max_body(N)]` y endpoints `#[raw]` en el macro. Con esto **el
framework ya no bloquea nada**: A1, A2 y A3 cerrados.

- **`#[max_body(N)]` salió casi gratis**, como preveía el plan:
  `decompose_request` ya recibía el límite como parámetro y el macro le
  pasaba una constante. Acepta una expresión, así que
  `#[max_body(64 * 1024 * 1024)]` se lee como se dice.
- **`#[raw]` es un atributo hermano**, no un flag dentro de
  `#[get(...)]` como proponía el documento. Cambié de idea: así la
  gramática del atributo de ruta sigue siendo "una ruta, o nada", y el
  nuevo encaja con `#[middleware]` y `#[max_body]`.

Lo que importa del wrapper generado es el **orden**: parte la petición,
corre los middlewares sobre las `Parts`, y la reensambla con
`Request::from_parts`. De ahí salen las tres garantías que hacen que
`#[raw]` sea parte del framework y no un agujero en él — los guards
siguen corriendo, las extensiones sobreviven (así que los extractores de
axum funcionan dentro del handler), y un `RestError` sigue renderizando
el cuerpo de error del framework. Solo se saltan el buffering y la
serialización JSON.

Los path params van en las extensiones (`path_param(&req, "digest")`),
porque un handler crudo nunca ve la descomposición y
`/v2/:name/blobs/:digest` no sirve de nada si no puedes leer `digest`.

**Dos errores de compilación deliberados**, ambos verificados
compilándolos de verdad, no leyendo el código: `#[raw]` + `#[max_body]`
juntos (un endpoint crudo no bufferiza, así que un límite de bytes es
una creencia falsa sobre el código), y cualquiera de los dos sin
atributo de ruta — son `proc_macro_attribute` inertes por su cuenta, así
que el método compilaría y no se montaría nunca, en silencio.

**Lo que esto desbloquea en la Fase 2:** los blobs OCI en streaming
(push y pull), y SSE para logs de contenedor en vivo, sin salir del
sistema de controllers ni perder DI, middlewares y el harness de tests.

### 8.4 M0 — hecho

El esqueleto del producto. 29 tests, `fmt` y `clippy -D warnings`
limpios, y verificado ejecutando el binario de verdad, no solo en tests.

| | |
| --- | --- |
| `install` | layout (0700), `config.toml`, BD + migraciones, ledger |
| `serve` | plano de control sobre `ProjectRunner`, apagado grácil |
| `doctor` | configuración, esquema, y los 9 pasos del ledger con su estado |

**Medido en release:** binario de **4,2 MB**, **7 MB de RSS** tras 50
peticiones. El presupuesto del §1 era < 40 MB, así que hay holgura de
sobra para lo que viene.

**Cuatro decisiones que no estaban en el plan**

1. **El plano de control escucha en `127.0.0.1`, no en `0.0.0.0`.**
   Todavía no hay autenticación, y un API sin auth expuesto a la red
   no es un default aceptable por la comodidad de un hito. Cambia
   cuando llegue el edge, que sí termina TLS y autentica.

2. **`deny_unknown_fields` en todo el `config.toml`.** Una errata deja
   al operador creyendo que cambió algo — el peor fallo posible, porque
   todo *parece* bien. El coste es que una sección de una feature que
   aún no existe se rechaza, lo cual es honesto: se iba a ignorar
   igualmente.

3. **`/healthz` y `/readyz` separados.** El primero no toca nada; el
   segundo hace round-trip a la BD. Juntarlos es cómo un nodo con la
   base de datos inalcanzable sigue recibiendo tráfico: la pregunta que
   se le hace es "¿estás vivo?" y lo está.

4. **El ledger existe desde M0 con los 9 pasos declarados**, aunque
   solo 3 estén implementados. `doctor` lista los demás como
   `not implemented yet` y **no los cuenta como problemas** — un plan
   que se ve vale más que uno que hay que inferir, y un nodo sano no
   debe parecer roto.

**Dos cosas que quité por especulativas.** Escribí una tabla `setting`
con sus helpers y un `open_in_memory` público, y nada los usaba: los
warnings de dead code lo dijeron antes que yo. La tabla vuelve en la
migración que añada lo primero que la necesite. Escribir el schema
completo del §4 de una vez habría sido escribir código sin usuario.

**Un hueco que dejé abierto a propósito**, anotado en `api.rs`: no hay
test de que `/readyz` devuelva 503 con la BD caída. SQLite sigue
respondiendo desde un handle abierto aunque borres el fichero, así que
montarlo exige inyección de fallos — más maquinaria que las tres líneas
que cubriría. Un test que asserte "200 o 503" sería cobertura falsa.

**Y una cosa que le faltaba al framework**, encontrada al escribir el
primer consumidor: `wabot-feature-sqlite` no reexportaba `rusqlite`,
pero su API pública lo exige. Corregido allí.

**Nota para M1:** `serve` escucha en `edge.https_port` en texto plano.
En desarrollo se baja con `WABOT_DEPLOY_HTTPS_PORT=3000`; en producción
443 necesita root. El listener lo reemplaza el edge.

### 8.5 M1 — hecho

El edge. 55 tests, `fmt` y `clippy -D warnings` limpios, verificado con
TLS real: `curl` sin `-k`, confiando solo en la CA exportada, valida la
cadena. RSS release **8,7 MB**.

**La simplificación que cambió el diseño.** El §4 planteaba escribir el
accept loop a mano con `hyper::server::conn::auto`. No hace falta: F2 ya
termina TLS con resolver dinámico, maneja upgrades y drena. Así que el
edge es **un `axum::Router` cuyo fallback despacha por `Host`**. Eso
borra de este crate un accept loop, una implementación de apagado
grácil y un rastreador de conexiones — y garantiza que el edge no se
desvíe del framework en cómo se sirve una conexión.

**Un bug real que cazó un test.** Las firmas ECDSA son aleatorizadas, así
que reconstruir la CA con `from_ca_cert_pem(...).self_signed()`
**re-firma** y produce bytes distintos en cada arranque: mismo TBS,
firma nueva. El fingerprint que el operador confía habría cambiado en
cada reinicio. `LocalCa` ahora lleva el PEM almacenado (lo que se
entrega) separado del `Certificate` reconstruido (solo para firmar
hojas).

**Decisiones que importan**

- **Despacho por `Host`, no por SNI** — y leyendo `:authority` *y* el
  header, porque HTTP/2 usa el primero y HTTP/1.1 el segundo. Leer solo
  uno da un edge que funciona en una versión del protocolo y da 404 en
  la otra.
- **Sin rutas configuradas, todo llega al plano de control.** Un nodo
  recién instalado tiene que ser alcanzable en la dirección que sea; que
  la primera petición tras `install` fuera un 404 sería absurdo.
- **La CA se exporta a `certs/local-ca.crt`**, no se imprime. Un PEM en
  el scrollback es algo que copiar con cuidado; una ruta es algo que se
  le pasa a `security add-trusted-cert` o `update-ca-certificates`.
- **Los nombres del certificado se guardan en columna**, no se decodifican
  del DER. Decidir si reemitir es una comparación de conjuntos, y un SAN
  de IP nunca aparecería como el texto que se pidió.
- **Redirect 308, no 302** — el método y el cuerpo tienen que sobrevivir,
  o un POST se convierte en GET camino de HTTPS. Y el puerto se
  reconstruye: arrastrar el de origen crea un bucle.
- **`bind_address` es `0.0.0.0` solo en puertos privilegiados.** En
  puertos altos —o sea, un desarrollador— escucha en loopback: una
  consola sin autenticación no debe aparecer en la red del portátil
  donde se arrancó.

**El proxy y los upgrades.** Un proxy que solo reenvía cuerpos parece
terminado y rompe todos los WebSockets. Hay `hyper::upgrade::on` en las
**dos** patas y `copy_bidirectional` entre ellas. El test
`an_upgraded_connection_is_proxied_both_ways` levanta sockets de verdad
(el `RestHarness` no sirve: usa `oneshot`, y un upgrade es justo lo que
sobrevive a la respuesta) y comprueba que los bytes vuelven. Sin
librería de WebSocket: al proxy le da igual el protocolo destino, lo que
importa es relevar el 101 y unir los streams.

También se limpian las cabeceras hop-by-hop **incluidas las que liste
`Connection`** — un proxy que solo quita el conjunto fijo filtra el
resto — y se preserva el `Host` del cliente, porque muchas apps enrutan
con él.

**Lo que quité por no tener usuario:** `routes::upsert` quedó
`#[cfg(test)]`. Nada escribe rutas hasta que existan los despliegues en
M3.

**Una dependencia que no estaba prevista:** `rcgen` con feature
`x509-parser`, necesaria para `from_ca_cert_pem`. La alternativa
—reconstruir los parámetros de la CA de memoria— produce un certificado
distinto del guardado, y discutir si la cadena sigue validando es peor
que una dependencia.

### 8.6 M2 — ACME

Certificados reales de Let's Encrypt por HTTP-01, renovados en
background. 63 tests, `fmt` y `clippy -D warnings` limpios.

**Dos bugs que encontraron los tests, y el segundo era de producción**

1. **Los tests hablaban con Let's Encrypt.** Al cablear ACME en
   `install`, los tests de instalación empezaron a pedir certificados
   para `node.example.com` — un dominio que no es nuestro — contra
   producción. Lento, inestable, y gastando el rate limit de alguien
   para que le digan que no. El helper `config_in` de los tests ahora
   pone `acme.disabled = true`, con el porqué escrito al lado.

2. **`instant-acme` trae `aws-lc-rs` por defecto; el framework fija
   `ring`.** Con los dos providers activos, rustls se niega a elegir y
   **panica en el primer uso** — o sea, `install --domain` habría
   reventado en un nodo real, no solo en tests. Además aws-lc-rs
   necesita cmake, lo que rompería la propiedad de "compila en una
   máquina pelada" por la que el framework eligió `ring`. Resuelto con
   `default-features = false` + feature `ring`, que propaga también a
   hyper-rustls: un solo backend criptográfico en el binario.

**Un bug que solo apareció en el nodo real, y era del tipo peor**

Probé contra staging (funcionó), cambié a producción, reinicié — y el
nodo **siguió sirviendo el certificado de staging**. Sin error, sin
aviso, sin nada en el log. Todo parecía bien y ningún navegador
confiaba en el sitio.

La causa era mía: `existing.issuer.starts_with("acme")` matcheaba tanto
`acme` como `acme-staging`, así que el certificado de staging pasaba
por "ya tenemos uno de una autoridad ACME y no ha caducado". La
corrección es guardar **la URL del directorio** como emisor y comparar
exacto — que además es la representación honesta: "este certificado
vino de esta autoridad".

Lo que ningún test local habría cazado, porque el escenario es un
cambio de configuración entre dos ejecuciones. El test de regresión que
añadí sí lo caza: verifiqué que **falla con el bug reintroducido**
(`Ok(false)`, el síntoma silencioso exacto) y pasa con la corrección.

**Decisiones**

- **Producción por defecto**, `--acme-staging` para probar. Un default
  equivocado aquí es un certificado que ningún navegador confía.
- **El challenge vive en la BD, no en memoria.** Una orden puede estar
  en vuelo cuando el nodo se actualiza, y un 404 después de eso la
  falla por una razón que ningún log explica.
- **La ruta del challenge se resuelve antes del fallback que
  redirige.** La validación HTTP-01 es una petición en texto plano; un
  308 a HTTPS mandaría a la autoridad contra un certificado que todavía
  no existe. Hay un test que lo fija.
- **`install` intenta y avisa; nunca aborta.** Un DNS que aún no
  propagó es mala razón para quedarse sin nodo. `serve` reintenta con
  backoff de 1 min a 6 h, y una vez obtenido comprueba cada 12 h.
- **La caducidad se lee del certificado**, no se asume. Let's Encrypt
  emite a 90 días hoy y ha dicho que lo acortará; un calendario de
  renovación construido sobre una suposición es uno que deja de renovar
  a tiempo.
- **La cuenta se guarda por directory URL**, así que staging y
  producción conviven y cambiar entre ellos es configuración, no una
  pérdida.
- **`doctor` muestra el último fallo de ACME**, que se guarda en la fila
  del certificado — para que la razón esté donde el operador mira, no
  en el journal.

**Verificado contra Let's Encrypt de verdad**, en una VM Ubuntu 26.04
con `wabot-deploy-testing.dev.tobaw.shop`:

| | |
| --- | --- |
| `install` sin listener | orden `Invalid`, aviso claro, **exit 0** — no aborta |
| `serve` | obtiene el certificado y lo instala **sin reiniciar** |
| staging → producción | detecta el cambio de autoridad y reemite |
| `curl` sin `-k` | `HTTP 200 · TLS 0` — cadena válida, emisor `CN=YE2` |
| reinicio | **0 llamadas ACME** — el rate limit protegido |
| SNI | el dominio recibe el de Let's Encrypt; `localhost` y sin-SNI, el local |
| RSS | 12 MB |

**Herramientas nuevas:** `scripts/deploy.sh` compila en el nodo por SSH
—más rápido que emular la arquitectura en local, y elimina la clase
entera de "el binario no corre allí"—, y `scripts/build-linux.sh` usa
Docker para un objetivo sin toolchain. Este último resultó impráctico
en Apple Silicon: 30 min emulando amd64 y un error de E/S de Docker
Desktop.

**Dos cosas del entorno que anotar para M3.** La VM tiene 843 MB sin
swap, y compilar Rust con LTO allí muere por OOM: hay 2 GB de swap
añadida a mano, sin persistir en `/etc/fstab`. Y el `drain_timeout` por
defecto es de 3 s porque `NODE_ENV` no dice `production` — la unit de
systemd debería fijarlo.

**Verificación end-to-end de M3/M4** — en VM efímera, no en la máquina de
desarrollo:

```sh
multipass launch 24.04 --name wd --memory 2G --disk 20G
multipass transfer target/release/wabot-deploy wd:/tmp/
multipass exec wd -- sudo /tmp/wabot-deploy install --domain <dns> --email <e> --acme-staging
multipass exec wd -- sudo /tmp/wabot-deploy install     # debe converger sin cambios
multipass exec wd -- systemctl status wabot-deploy containerd
multipass exec wd -- sudo ctr version && crun --version
multipass exec wd -- ps -o rss= -C wabot-deploy         # objetivo < 40 MB
curl -v https://<dns>/healthz                            # cadena válida
```

Tests de framework con los harnesses existentes (`RestHarness`,
`AsyncHarness`); tests del edge levantando el listener en un puerto efímero
con cert autofirmado — sin root y sin containerd.

### 8.7 M4 — cuentas, proyectos y servicios

La consola deja de ser una página de estado: registro del administrador,
proyectos, servicios y las pantallas para crearlos y borrarlos.
190 tests, `fmt` y `clippy -D warnings` limpios.

**El registro inicial: token impreso por `install`, no primer-visitante**

La alternativa obvia —el primero que abra la web crea la cuenta— es una
carrera que pierde el operador: entre que el nodo levanta y él abre el
navegador, cualquiera que conozca el dominio puede quedárselo. El token
lo imprime `install` en la terminal donde ya está el operador, se guarda
**hasheado**, se gasta al usarse y caduca en 24 h. `wabot-deploy
setup-token` emite otro para el token que expiró o la terminal que se
cerró; si ya hay administrador, se niega y lo dice.

**Un namespace de containerd, no uno por proyecto**

La separación por proyecto es de *nombres y etiquetas*, no de namespaces.
Un namespace por proyecto duplica el content store —la misma imagen base
bajada N veces— que es justo lo que el registry compartido de §7.1 existe
para evitar. El id de contenedor es `{proyecto}--{servicio}` y las
etiquetas llevan el proyecto, así que listar, reconciliar y limpiar por
proyecto siguen siendo una consulta. La red CNI por proyecto llega
después de que los servicios corran: aislar tráfico entre proyectos que
todavía no tienen tráfico es orden equivocado.

**Formularios planos, sin JavaScript**

POST y 303. Funciona con el scripting apagado, y la defensa CSRF es
`SameSite=Lax` en la cookie de sesión en vez de un token que enhebrar por
cada formulario. Los POST son endpoints `#[raw]` —lo que un formulario
necesita de vuelta es un 303 con `Set-Cookie`, y el camino JSON no
expresa ninguna de las dos cosas—. Los errores vuelven en un parámetro de
query: un formulario rechazado no es secreto, y así no hay estado con
tiempo de vida.

**El middleware anota, no rechaza**

`Middleware` en el framework es solo-rechazo, y un rechazo es un cuerpo
JSON. Para un navegador eso es la respuesta equivocada: quien no tiene
sesión quiere la página de login, no un 401 con `{"error":…}`. Así que
el middleware siempre tiene éxito —lee la cookie y, si nombra una sesión
viva, asigna la cuenta a `Auth`— y cada vista decide. La vista puede
devolver `ViewOutcome::Redirect`; un `RestError` no.

Consecuencia: **el POST es la frontera, no la página**. Cada endpoint que
muta vuelve a comprobar la sesión, porque un formulario lo envía
cualquiera. Hay un test por endpoint que lo fija.

**Un bug del framework, y del tipo silencioso**

Con la sesión creada y la cookie correcta, la consola seguía mandando a
`/sign-in`. El middleware recibía **cero cabeceras**: el macro `#[view]`
le pasaba a `produce` un head *sintético* reconstruido desde la URI y el
flag de navegación, guardándose el real solo para la respuesta.

O sea: **cualquier middleware sobre una vista veía una petición sin
cabeceras**. Un guard de cookie o de bearer no encontraba nada, nunca, y
como los middlewares son solo-rechazo, la forma de no encontrar nada era
dejar pasar a todo el mundo. Nada fallaba en voz alta; la página
renderizaba.

`Parts` implementa `Clone`, así que la copia sintética no hacía falta:
ahora `produce` recibe un clon del head real. Test de regresión en
`ui_http.rs`, **validado reintroduciendo el bug** — falla con
`["<no header>"]` frente a `["jorge"]`.

**Cambios de framework**

| Cambio | Por qué |
| --- | --- |
| `#[view]` recibe el head real | lo de arriba: un middleware de UI no veía nada |
| `ViewOutcome` en el prelude | se exportaba `Redirect` sin el tipo que lo transporta |
| `RequestBuilder::form()` en el harness | un valor con `&` o espacio tiene que llegar entero, y un cuerpo a mano es donde eso deja de ser cierto |

**Decisiones del dominio**

- **`Account` es también las claims.** Dos structs de la misma forma es
  un segundo sitio donde divergir; lo cazó el compilador la primera vez
  que el header de layout recibió el equivocado.
- **La contraseña no se recorta.** Los espacios al principio y al final
  son parte de ella, y quitarlos en silencio produce una contraseña que
  no se puede volver a teclear.
- **Usuario desconocido y contraseña mala responden igual**, y el camino
  del desconocido paga el mismo hash. Hay un test que compara las dos
  respuestas.
- **El env se parte por el *primer* `=`.** La mayoría de los valores que
  alguien pega son URLs de conexión o blobs base64 con `=` dentro;
  partir por todos trunca exactamente los secretos que peor sienta
  truncar.
- **Borrar es el par natural de crear.** Sin ello, un servicio mal
  tecleado es permanente. Zona de peligro, POST, sin diálogo de
  confirmación —un diálogo necesita JavaScript—.
- **El badge de estado dice "Pending", no "Running".** Todavía nada
  arranca contenedores: el estado deseado es una intención declarada, y
  la interfaz dice cuál de las dos cosas está mirando.

**Dos cosas que solo aparecieron en el nodo**

1. **`install` dejaba corriendo el binario viejo.** El paso `Start`
   estaba en el ledger, así que tras un despliegue el proceso anterior
   seguía sirviendo mientras el código nuevo esperaba en disco. Sin
   error, sin aviso: la consola simplemente no tenía las páginas
   nuevas. Misma clase que el bug de `runtime::ensure` de M3 —el paso
   preguntaba por la historia de la ejecución en vez de por la cosa—.
   Ahora compara el inodo de `/proc/<pid>/exe` con el del binario
   instalado: `install_binary` renombra encima, así que el proceso
   viejo se queda con un inodo desvinculado y la diferencia es exacta.

2. **`install` quemaba un token todavía válido.** Ejecutarlo dos veces
   invalidaba el token que la primera ejecución acababa de imprimir:
   converger rompiendo justo lo que te habían dado. Ahora solo emite si
   no hay uno vigente.

**Verificado en vivo** contra `wabot-deploy-testing.dev.tobaw.shop`, con
el certificado real de Let's Encrypt y sin `-k`:

| | |
| --- | --- |
| `/` sin sesión | 302 a `/setup` |
| setup con token bueno | 303 a `/`, con `Set-Cookie` |
| `/` con sesión | lista de proyectos, cuenta en la cabecera, emisor `letsencrypt` |
| crear proyecto / servicio | 303 a la página del proyecto; imagen, puerto y badge correctos |
| imagen inválida | vuelve al formulario con el motivo en la query |
| POST sin sesión | 303 a `/sign-in`, y nada creado |
| borrar servicio | 303, y el proyecto queda vacío |
| sign-out | cookie caducada y la sesión revocada en el servidor |
| segundo `install` | "already running this binary", token intacto |
| RSS | 14,4 MB |

### 8.8 M5 — los contenedores corren de verdad

De una fila a un contenedor: red CNI por proyecto, despliegue al crear,
parar/arrancar, y reconciliación al arrancar el nodo. 210 tests, `fmt` y
`clippy -D warnings` limpios.

**Por qué la red llegó ahora y no después**

El plan era servicios primero, CNI después. Pero con el netns del host
compartido, `container_port` no significa nada: el proceso escucha en un
puerto *del nodo*, así que dos proyectos no pueden usar el 8080 y
`nginx:alpine` —que escucha en el 80 porque lo dice su config— choca con
el edge. Un contenedor que hay que modificar para poder desplegarlo no es
una plataforma de despliegue. Así que la red vino con el despliegue, no
después.

Un bridge por proyecto (`wd-<n>`) y un `/24` dentro de `10.42.0.0/16`.
Contenedores del mismo proyecto se ven entre sí; de proyectos distintos
no, porque son dominios L2 separados y nada rutea entre ellos. El índice
se asigna en el primer despliegue, no al crear el proyecto: hay 254, y un
proyecto que no corre nada no debería tener uno.

Invocamos los plugins directamente en vez de usar una librería CNI: el
protocolo es "ejecuta este binario con cinco variables de entorno y el
config por stdin", y una librería para eso es más dependencia que código.

**Observado, no recordado**

El badge sale de preguntarle a containerd, no de una columna que alguien
escribió al pulsar un botón. Un nodo que reporta lo que le dijeron es un
nodo que miente después del primer crash. `Unknown` es un estado propio y
distinto de `Not deployed`: "el runtime no contesta" y "esto no está
desplegado" son problemas distintos, y confundirlos manda a alguien a
redesplegar un servicio sano.

Reconciliar usa el mismo código que desplegar. Arrancar no tiene camino
propio: pregunta, por cada servicio que debería estar corriendo, si lo
está, y despliega los que no. Solo arranca cosas — un contenedor que
ninguna fila reclama se deja en paz y se reporta, porque borrar lo que el
nodo no entiende es como un reconciliador destruye datos.

**Cuatro fallos, todos en el nodo, ninguno reproducible en local**

1. **El ledger otra vez, y por tercera vez.** `Step::Runtime` estaba
   marcado hecho, así que un nodo que ya tenía containerd nunca recibió
   los plugins CNI que se añadieron a ese mismo paso. La corrección no es
   otro parche: **el ledger registra, no decide**. Todos estos pasos son
   convergentes por dentro —cada uno pregunta por la cosa— así que
   saltárselos porque una ejecución anterior dijo "hecho" solo produce
   respuestas viejas.

2. **`project--service` no es un id que containerd acepte.** Su regex
   —`^[A-Za-z0-9]+(?:[._-][A-Za-z0-9]+)*$`— permite separadores simples.
   Ahora es `project.service`: un punto no puede aparecer dentro de un
   slug, así que el id además se parte de vuelta sin ambigüedad, cosa que
   con `-` no pasaría.

3. **`setns: Invalid argument`, que no nombra ni a systemd ni a la
   propagación.** `PrivateTmp`, `ProtectHome` y `ProtectSystem` ponen la
   unidad en su propio mount namespace, y systemd lo deja **esclavo** del
   host: los montajes entran, no salen. El bind mount de `/run/netns/<id>`
   se quedaba dentro, y el shim de containerd —en otra unidad— veía solo
   el fichero vacío que `ip netns` deja debajo. `MountFlags=shared` no lo
   arregla; sigue siendo esclavo. Un demonio cuyo trabajo es construir
   namespaces y montajes para la máquina no puede estar escondido del
   árbol de montajes de la máquina. Hay un test que impide que vuelvan.

4. **DNS que funciona para `wget` y no para `nslookup`.** El host daba
   cuatro resolvers, los dos primeros IPv6, y el bridge del proyecto es
   solo IPv4. `wget` recorre la lista y cae en los IPv4; `nslookup` prueba
   el primero y dice "Network unreachable". Una lista de resolvers donde
   el DNS funciona para unos programas y no para otros es peor que una
   más corta.

**Verificado en el nodo:**

| | |
| --- | --- |
| `install` | descarga los plugins CNI 1.9.1 (checksum publicado) y activa `ip_forward` |
| reconciliar | levanta el servicio al arrancar: `attached 10.42.1.7 bridge=wd-1`, `deployed pid=…` |
| HTTP al contenedor | `200` en 1,5 ms desde el host |
| DNS dentro | `nslookup github.com` → `140.82.114.3` |
| salida a internet | `wget https://example.com` desde el contenedor |
| reinicio del nodo | el contenedor **no** se reinicia: reconcile lo ve corriendo y lo deja |
| fugas | 1 netns, 1 reserva de IP, 1 snapshot activo — nada de los despliegues fallidos |
| RSS | nodo 11,7 MB + shim 11,1 MB por contenedor |

**Lo que todavía no hay:** ruta pública a un servicio. El contenedor se
alcanza desde el nodo, no desde fuera — falta decidir el esquema de
hostname y emitir certificado por ruta, que es el siguiente corte.

### 8.9 M6 — puertos, dominios y HTTPS por servicio

Un servicio ya no tiene "un puerto": tiene los que declare, y cada uno
dice si sale al exterior como TCP crudo, si sirve HTTPS en un hostname,
las dos cosas o ninguna. 240 tests, `fmt` y `clippy -D warnings` limpios.

**El modelo: una fila por puerto, dos columnas nullable**

`container_port` es lo que el proceso escucha dentro. `host_port` no
nulo significa publicado en la IP del nodo; `hostname` no nulo significa
que responde HTTPS ahí. Las cuatro combinaciones salen sin casos
especiales, y la respuesta común a las dos últimas es "no": la mayoría
de los servicios no exponen nada, y eso debía ser el estado por defecto
en vez de una columna que había que dejar vacía.

La columna `container_port` del servicio conflacionaba tres preguntas
distintas. La migración la mueve a una fila y la borra.

**El dominio: comprobar antes de aceptar, no después**

Un hostname se verifica contra DNS *antes* de escribirse. La
alternativa —guardarlo y enterarse al pedir el certificado— es un
servicio que parece configurado, no responde, y la razón está en un log
que nadie mira.

La comparación es contra el **dominio del nodo**, no contra su IP: el
nodo no sabe de forma fiable su dirección pública —detrás de NAT ve una
privada, y preguntárselo a un tercero es depender del uptime de otro—
pero su propio dominio ya resuelve a donde el mundo lo alcanza. Dos
lookups y ninguna suposición. Se intersecan los conjuntos en vez de
compararlos enteros: un nombre con dos registros A sigue llegando aquí
si alguno coincide.

El wildcard se detecta sondeando una etiqueta aleatoria, porque un
registro comodín es invisible a una consulta directa: solo responde por
nombres que nada más define. Si `wd-probe-xxxx.<dominio>` resuelve donde
resuelve el dominio, hay comodín y se propone
`<servicio>.<proyecto>.<dominio>` ya rellenado. Si no, el formulario
explica qué falta y pide un hostname, que se comprueba igual.

**Las rutas se derivan, no se editan**

Son función de los puertos, los servicios y sus direcciones. Cualquier
otra cosa deriva: un servicio redesplegado a otra dirección deja una
ruta apuntando a la anterior, y el fallo es una página que no carga sin
nada en ningún log. Así que el conjunto entero se recalcula tras cada
despliegue.

Una trampa que el test fija: el edge sirve el control plane en todos los
hostnames **solo mientras la tabla está vacía**, así que la primera ruta
de servicio habría dejado la consola inalcanzable —en el dominio del
nodo, que es justo donde alguien iría a deshacerlo—. La sincronización
escribe siempre las filas del control plane.

**Un certificado por hostname**, no uno con muchos nombres: HTTP-01 no
emite comodines, y un certificado multi-nombre habría que reemitirlo
—y podría fallar entero— cada vez que un servicio cambia de dominio. Un
fallo en un nombre no impide los demás.

**Tres fallos, los tres en el nodo**

1. **El proxy hablaba HTTP/2 a un contenedor que solo sabe HTTP/1.1.**
   El navegador negocia h2 con el edge y la versión de la petición se
   reenviaba tal cual: `client error (UserUnsupportedVersion)`, que no
   nombra ninguno de los dos extremos. Los dos saltos son negociaciones
   separadas — eso es justo lo que permite terminar HTTP/2 delante de
   una aplicación que nunca aprendió más que HTTP/1.1.

2. **`portmap DEL` con lista vacía no borra nada.** El plugin elimina la
   cadena entera del contenedor —indexa por id y por red, no por lo que
   diga la lista— pero valida el config antes de llegar ahí, y una lista
   vacía es un no-op. El caso que más necesita limpieza es justo el que
   la deja vacía: un servicio que dejaba de publicar un puerto conservaba
   la regla DNAT mandando el puerto del nodo a la dirección que el
   contenedor tuvo la última vez. Comprobado en el nodo que un DEL
   nombrando un mapping que nunca existió borra la cadena igual.

3. **Y el puerto de relleno era `0`**, que no es un puerto: el plugin
   parsea el config antes de tocar nada y lo rechazaba. Fallo silencioso
   de la especie habitual — un warning en el journal, una regla de más, y
   un puerto del nodo respondiendo por un contenedor que ya se había
   movido.

**Verificado en el nodo, de fuera hacia dentro:**

| | |
| --- | --- |
| wildcard | `wd-probe-*.<dominio>` resuelve a la IP del nodo |
| certificado del servicio | emitido en 3 s e instalado **sin reiniciar** |
| `https://nginx.first-project.<dominio>` | `200`, TLS válido, `server: nginx/1.31.3` |
| TCP publicado | `http://<ip>:20001` → `200` desde fuera |
| consola | sigue respondiendo en el dominio del nodo |
| quitar el puerto | la regla DNAT desaparece; 0 reglas residuales |

**Límite conocido:** reconciliar solo mira si el contenedor corre, no si
sus mappings coinciden con las filas. Un cambio de puertos que falló al
desplegarse y luego un reinicio dejan el contenedor con la configuración
anterior hasta el siguiente despliegue explícito.

### 8.10 M7 — la consola con marco, y el nodo con cifras

Topbar, side nav y selector de proyecto con la misma forma que wabot
console, más una página de nodos con el desglose de memoria en vivo.
259 tests, `fmt` y `clippy -D warnings` limpios.

**El marco lleva datos propios, no prestados**

`Frame` copia lo que necesita —usuario, la lista de proyectos como
(slug, nombre), el proyecto actual, la ruta— en vez de tomarlos
prestados. El motivo es concreto: el marco nombra el proyecto actual en
la navegación mientras la página dibuja ese mismo proyecto, y el cierre
que construye `rsx!` captura por movimiento. Dos préstamos del mismo
valor es una pelea que gana el compilador; para un puñado de cadenas
cortas, copiar sale más barato que el lifetime.

El cuerpo se renderiza antes que el marco por la misma razón: así el
préstamo de la página termina antes de que el marco tome el suyo.

**El selector es un formulario, no un desplegable que navega**

Un `<select>` que enruta al cambiar necesita JavaScript. Dentro de un
formulario con botón funciona sin nada. Y no hay "proyecto
seleccionado" guardado en ninguna parte: **la URL es la selección**, que
es lo que hace que un enlace a un proyecto signifique lo mismo para
todos.

**Un nodo, en plural**

Hay exactamente uno —este— y la consola lo lista como una lista de uno.
No es decoración: la forma de la página y la de los datos son las que
recibiría un segundo nodo, y una lista que empieza siendo una página de
detalle no se convierte en lista sin romper todos los enlaces que
apuntaban a ella.

**La memoria, atribuida en vez de sumada**

"Usada" es un número que muestra cualquier herramienta y sobre el que
nadie puede actuar. Lo que necesita quien opera *este* nodo es qué parte
es la plataforma y qué parte es lo que desplegó — porque la parte de la
plataforma es justo el número que este producto intenta mantener
pequeño.

Cinco líneas: wabot-deploy, containerd, los shims (uno por contenedor
corriendo), los contenedores, y todo lo demás. Los shims van aparte y no
metidos en ninguno de los dos lados: existen *porque* hay un contenedor,
pero son coste del runtime, no de la imagen, y esconderlos en cualquiera
de las dos columnas respondería a una pregunta que nadie hizo.

**Las partes no cuadran, y decirlo es el punto.** El `memory.current` de
un contenedor incluye su page cache, que también cuenta en el `Cached`
del sistema; el RSS de dos procesos cuenta dos veces las páginas que
comparten. Así que "todo lo demás" es un resto, no una medida. La
alternativa es un número que cuadra exacto y no significa nada: lo que
hace falta es el orden de magnitud de cada parte —si la plataforma
cuesta 30 MB o 300— y para eso el solape en la tercera cifra no es el
problema. La página lo dice en voz alta debajo de la tabla.

Se lee de `/proc` y del árbol de cgroups, sin crate intermedio: son
interfaces estables del kernel y parsear las cuatro líneas que hacen
falta es más corto que la dependencia que parsearía todas. El cgroup de
un contenedor se encuentra por el pid de su tarea, no adivinando la
ruta: el layout depende del driver de cgroups y la adivinanza fallaría
justo en las máquinas configuradas de forma poco habitual.

**El primer JavaScript del producto**, y es todo el que hay: un
`EventSource` que sustituye las cifras en su sitio cada dos segundos. La
página renderiza completa y correcta sin él —el script solo reemplaza
texto que ya está— así que la consola sigue sirviendo en la máquina
donde el stream no se puede abrir, que es justo la máquina desde la que
alguien mira esta página. Las cifras las formatea el servidor en los dos
casos, así que la primera pintura y cada actualización no pueden
discrepar; hay un test que fija que cada celda del stream existe en la
página.

**Comprobado en el nodo**: 843 MB totales, wabot-deploy 13 MB,
containerd 20 MB, shim 7,8 MB, contenedor 376 kB en reposo — y 3,1 MB
tras 40 peticiones, que es la lectura siguiendo a la realidad. El stream
responde 401 sin sesión.

**Un fallo que solo aparece al cambiar un servicio en caliente**

Dar un hostname a un servicio creaba la ruta y **nadie pedía el
certificado**. El bucle ACME duerme 12 h una vez asentado, así que el
nombre quedaba enrutado y sin TLS durante medio día: la consola lo
mostraba configurado y ningún navegador podía abrirlo.

La corrección es un timbre —`Wake`, un `Notify` compartido— en vez de
emitir en línea desde el handler: la emisión pertenece al bucle, que es
quien tiene los reintentos y el backoff, y hacerlo en la petición
significaría o bloquear el ida y vuelta con la autoridad o inventar una
segunda política de reintentos al lado de la que ya hay. Al despertarse,
el backoff vuelve al mínimo, porque un nombre añadido hace un segundo es
justo el caso donde el DNS puede estar aún asentándose.

Y la página dice cuál es el estado: un puerto HTTPS cuyo certificado
todavía no está lleva "Certificate on the way". Se pregunta, no se
supone — el certificado llega segundos después de la ruta, y esa ventana
es exactamente cuando alguien está mirando.

**Verificado, cambio a cambio, contra el nodo:**

| cambio | qué debe pasar | resultado |
| --- | --- | --- |
| añadir puerto HTTPS con nombre nuevo | certificado y ruta | challenge y certificado en **7 s**, `TLS 0` |
| añadir un segundo puerto | el contenedor se redespliega y **las rutas anteriores siguen** | `.17 → .18` en las tres rutas, HTTPS 200 |
| publicar TCP | una regla DNAT, a la dirección actual | una sola, `20000 → 10.42.1.18:8081` |
| quitar el puerto publicado | la regla desaparece | 0 reglas |
| quitar un puerto HTTPS | su nombre deja de responder, los demás no | 404 el retirado, 200 los otros |
| parar el servicio | sin rutas, sin netns, sin tareas | 0/0/0, el nombre da 404 |
| arrancarlo otra vez | todo vuelve | 200 |

### 8.11 M8 — personas y permisos

Dos niveles de rol, invitaciones por enlace, y la consola aplicándolos
en cada página y cada POST. 296 tests, `fmt` y `clippy -D warnings`
limpios.

**Dos niveles, porque hay dos clases de pregunta**

"¿Puede esta persona crear proyectos, invitar, mirar la memoria del
nodo?" es del nodo. "¿Puede desplegar *aquí*?" es de un proyecto, y la
respuesta cambia según el proyecto — que es justo la razón de que
existan los proyectos.

Nodo: `admin` o `member`. Proyecto: `owner`, `deployer`, `viewer`.
**Un administrador no es miembro de nada y llega a todo**: la
pertenencia responde la pregunta de proyecto y un admin nunca llega a
esa pregunta. Es deliberado — un nodo donde el operador tiene que
añadirse a un proyecto para poder arreglarlo es un nodo que lo deja
fuera de lo que opera.

**Un rol desconocido es el rol más pequeño.** Toda lectura cae al
mínimo privilegio que podría significar: una fila escrita por una
versión que conoce un rol que esta no, no puede conceder más de lo que
esta entiende. Fallar hacia arriba una sola vez es una concesión
silenciosa; fallar hacia abajo es una negativa que alguien reporta.

**Las decisiones son valores, no condiciones.** Toda comprobación
devuelve un `Access` y cada llamada pregunta por nombre —
`may_deploy()`, no `role == Deployer || role == Owner || es_admin`. La
comparación escrita en cada sitio es exactamente cómo una de treinta
acaba sin una cláusula.

**El filtro es la consulta, no la página.** La lista de proyectos de un
miembro sale de un JOIN con `membership`, no de "todos, filtrados
después". Una lista construida con todo y estrechada más tarde se
escapa la primera vez que alguien añade una página que olvida
estrecharla. Y "no es tuyo" responde **idéntico** a "no existe": si se
distinguieran, los nombres de todos los proyectos del nodo serían
enumerables preguntando uno a uno.

**El invitado elige su propia contraseña**

Un administrador que teclea la contraseña de un colega la conoce, y esa
contraseña viaja por el canal que hayan usado. Un enlace de invitación
no lleva contraseña: lleva el *derecho* a crear una cuenta, una vez,
antes de caducar. Mismos mecanismos que el token de setup — hasheado,
de un solo uso, con caducidad de siete días.

Dos detalles que se ganan pensando en el orden de las operaciones:

- **La cuenta se crea antes de gastar la invitación.** Un rechazo que el
  invitado puede arreglar —nombre ocupado, contraseña corta— no debe
  costarle el enlace.
- **Gastar es un `UPDATE … WHERE used_at IS NULL`**, y si no afecta a
  ninguna fila es que otro la usó entre la consulta y la escritura: la
  cuenta recién creada se borra, porque una cuenta creada por una
  invitación que no se gastó en ella no debería existir.

Y una invitación puede llevar proyecto y rol, así que el caso normal es
un enlace en vez de un enlace y un segundo paso que alguien olvida.

**Dos cosas que no se pueden hacer**, ambas porque la vuelta atrás
sería editar la base de datos a mano: quitar al único owner de un
proyecto, y degradar o borrar al único administrador del nodo.

**Verificado en el nodo**, con una invitación real:

| | |
| --- | --- |
| el enlace | dice a qué rol invita y pide usuario y contraseña |
| aceptar | crea la cuenta, la mete en el proyecto y **la deja con sesión** |
| lo que ve el miembro | solo `first-project`; `second-project` responde "no such project" |
| `/people`, `/nodes` | 302 a `/` |
| borrar el proyecto | rechazado: "only an owner can delete this project" |
| desplegar | permitido — es deployer |
| el enlace otra vez | "not valid" |

### 8.12 M9 — registry, releases y rollback

El nodo recibe imágenes, cada push es una release, y una release
anterior se vuelve a desplegar con un botón. 344 tests, `fmt` y
`clippy -D warnings` limpios.

**Compartir el content store, no copiar en él**

Los blobs entran directos al content store de containerd por su
servicio Content, y el manifest se convierte en un image record. Así
una imagen empujada **ya es** la imagen que el runtime corre: ni una
segunda copia en disco, ni un paso de importación, y `ctr images ls`
enseña lo que enseña la consola. Ésa es la razón de escribir esto en
vez de correr un registry en un contenedor al lado, que guardaría cada
capa dos veces en un nodo cuyo diseño entero va de no hacer eso.

La sesión de subida es de containerd, no nuestra: su escritura de
contenido es una transacción con nombre, reanudable por ref y offset.
Así que el id de la subida **es** el ref, y el módulo no guarda estado
entre peticiones — la alternativa (un fichero o un stream gRPC por
subida en un mapa) filtra uno cada vez que un cliente se marcha a
medias.

**Una release es un digest, no una etiqueta.** Las etiquetas se mueven:
"vuelve al latest de ayer" no significa nada cuando el latest de ayer
es el de hoy. Y la actual está *marcada*, no derivada de "la más
nueva", porque un rollback hace que la actual sea una vieja — que es la
función entera.

**Dos historiales, a propósito.** Volver atrás una imagen y volver atrás
una configuración son intenciones distintas: lo normal es "esta build
está mal, corre la anterior, mantén los ajustes que arreglé". Atarlas
haría una de las dos imposible.

**Cinco fallos, y ninguno se veía sin el nodo**

1. **El registry decía "ya lo tengo" de imágenes que nadie había
   empujado.** El índice de etiquetas era el de containerd, que
   comparte con todo lo demás: `ctr images tag` escribe uno, un pull
   escribe otro. El cliente se saltaba la subida, el push reportaba
   éxito, no se creaba ninguna release y nada decía por qué. Compartir
   el *contenido* es el diseño; compartir el *espacio de nombres de
   etiquetas* no lo era. Ahora el registry tiene su propia tabla.

2. **La referencia de la release no llevaba host**, así que containerd
   leía el primer segmento como un registry a marcar: un error de DNS
   sobre un host llamado `first-project`.

3. **Desplegar por digest no encontraba registro local**, porque solo
   creábamos el de la etiqueta. Ahora se crean los dos.

4. **Un push deja blobs, no snapshots.** Un contenedor necesita un
   rootfs desempaquetado, así que una imagen que llegó por push y nunca
   se bajó falla con "no unpacked layer" — que se lee como imagen
   corrupta y no como paso que falta. El servicio Transfer de containerd
   desempaqueta como parte de un *pull*; pedirle transferir una imagen
   del store a sí misma con configuración de unpack reporta éxito y no
   desempaqueta nada (probado en el nodo, dos veces, con plataforma y
   snapshotter escritos). Así que lo hacemos como lo hace un unpacker:
   por cada capa, preparar un snapshot sobre la cadena, aplicar el diff
   y comprometerlo bajo el chain ID que el runtime buscará.

5. **Cambiar una variable movía el servicio de imagen.** El redespliegue
   usaba la *referencia* del servicio —su etiqueta— en vez de la release
   que estaba corriendo. En cuanto la etiqueta se mueve, tocar el
   entorno traía lo que se hubiera empujado desde entonces: justo lo que
   las releases existen para evitar. Lo mismo pasaba al reconciliar tras
   un reinicio. Ahora ambos restauran la release marcada.

**Verificado en el nodo, con un cliente OCI real** (`ctr push` sobre
TLS, autenticado con token de push):

| | |
| --- | --- |
| `docker login` | acepta el token de proyecto como contraseña |
| push | blobs al content store compartido, image record y `registry_tag` |
| release | se registra sola, y se despliega sola por defecto |
| corre por digest | `…/first-project/nginx@sha256:4a73073b…` |
| push de una imagen mala | sale a producción: el servicio da 502 |
| rollback desde la consola | la release vieja vuelve a marcarse actual, **200** |
| variables | se guardan, se versionan, y restaurar una **no** cambia la imagen |

### 8.13 M10 — el framework publicado y el pipeline de release

`wabot 0.2.0` está en crates.io, así que el `[patch.crates-io]` que
apuntaba al checkout hermano desaparece: el proyecto resuelve el
framework desde el registro como cualquier otra dependencia. 345 tests
verdes contra el crate publicado.

Eso arregla de paso el CI, que llevaba roto sin que se notara: el patch
apuntaba a una ruta que en un runner no existe. Le faltaba además
`protoc` — la API de containerd llega como ficheros `.proto` y
prost-build lo invoca, así que el build fallaba dentro de una
dependencia, que es donde peor se lee.

**Un tag es el disparador, y el tag es la versión.** Nada más publica:
un release que puede salir desde una rama es uno que sale por
accidente. Y el workflow comprueba que el tag y el `Cargo.toml`
coincidan antes de construir — un `v0.3.0` construido de un árbol que
dice `0.2.0` produce un binario cuyo `--version` contradice la página
de la que se descargó, y eso no se descubre hasta que alguien está
depurando otra cosa.

**musl estático, no glibc.** Un build con glibc lleva como suelo la
versión del runner, y las máquinas donde esto aterriza no siempre son
más nuevas que un runner de GitHub. El coste es un asignador de memoria
más lento bajo mucha concurrencia; este nodo hace de proxy y escribe
SQLite, no es esa carga, y "copia este fichero a la máquina" es el
producto. Verificado en hardware real —el propio nodo, que es x86_64
Linux— antes de escribirlo como definitivo: compila, enlaza estático y
arranca.

Esa verificación cazó un fallo en el propio workflow: la comprobación
de "¿es estático?" buscaba la frase `not a dynamic executable`, y el
`ldd` de esa máquina dice `statically linked`. Habría rechazado
exactamente el binario que debía aceptar. Ahora busca bibliotecas
resueltas (`=>`), que es el hecho y no la redacción.

**Dos perfiles, y el segundo existe por experiencia.** `release` es LTO
completo con una unidad de codegen: el binario vive meses en una
máquina, así que minutos de enlazado se cambian por cada petición que
va a servir. `node` —el que usa `scripts/deploy.sh`— baja a LTO fino y
cuatro unidades, porque un build de release en una VM de un núcleo con
menos de dos gigas se lleva la máquina por delante: rustc tira de swap
hasta que sshd deja de contestar. Es algo que pasó, no algo que temer.
