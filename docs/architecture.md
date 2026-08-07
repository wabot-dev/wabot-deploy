# wabot-deploy — Arquitectura

Este documento describe **lo que el nodo hace hoy** y por qué está hecho
así. La §10 guarda el registro de decisiones por hito: lo que se decidió,
y —más útil— dónde la realidad contradijo el plan.

Premisa que ordena todo lo demás: **el framework es nuestro**. En cada
decisión la pregunta no es "¿cómo esquivo esta limitación?" sino "¿esto
es una capacidad genérica que le falta a wabot-rust, o es producto de
wabot-deploy?". Cuando es lo primero sube al framework y lo hereda
cualquier otra app; cuando es lo segundo se queda aquí.

---

## 1. Qué es

Un **binario único** que se instala en un nodo Linux y lo convierte en
una plataforma de despliegue de contenedores: el equivalente mononodo de
wabot-cloud + wabot-console, sobre infraestructura propia.

| wabot-cloud | wabot-deploy |
| --- | --- |
| Kubernetes multi-nodo (UpCloud) | containerd + crun, un nodo |
| Harbor | registry OCI embebido sobre el content store de containerd |
| ingress-nginx + cert-manager | edge embebido: TLS, routing por host, ACME |
| Postgres | SQLite |
| Node.js / TypeScript | Rust / wabot-rust |
| N procesos y pods de sistema | 2 procesos: `containerd` + `wabot-deploy` |

Objetivo transversal: **RAM mínima**. El presupuesto era < 40 MB RSS para
`wabot-deploy`; la medida en el nodo es **13 MB**, más ~20 MB de
containerd y ~11 MB de shim por contenedor corriendo (§9.2). El shim es
Go y no es nuestro.

Lo que sabe hacer:

- Instalarse solo: preflight, containerd + crun, plugins CNI, unidad de
  systemd, certificado y arranque.
- Terminar TLS con certificados de Let's Encrypt que obtiene y renueva
  solo, y enrutar por hostname a contenedores o a su propia consola.
- Correr contenedores con red por proyecto, puertos publicados y HTTPS
  por servicio.
- Recibir imágenes como registry OCI, convertir cada push en una release
  y volver a una anterior con un botón.
- Cuentas, invitaciones por enlace y permisos de nodo y de proyecto.
- Actualizarse a una release publicada, cuando alguien lo pide.

---

## 2. El reparto: framework vs producto

### 2.1 Lo que subió al framework

Ninguno de estos cambios es específico de containerd ni de despliegues.
Todos le faltaban a cualquier app wabot-rust que quisiera correr
self-hosted en una VM. Están en `wabot 0.2`, publicado en crates.io.

| # | Cambio | Por qué era del framework |
| --- | --- | --- |
| **F1** | `#[max_body(N)]` y endpoints `#[raw]` | 1 MiB fijo y siempre `(200, Json)` bloqueaba subidas, descargas y SSE |
| **F2** | TLS y apagado grácil en el servidor REST | ninguna app podía servir HTTPS |
| **F3** | `wabot-feature-sqlite` + `wabot-addon-async-sqlite` | `pg` era el único backend: sin esto no hay despliegue embebido ni tests sin base de datos externa |
| **F5** | Cancelación cooperativa y `sd_notify` | el `ProjectRunner` no entregaba señal de cancelación: la fase de drain no tenía a quién esperar |
| — | `#[view]` recibe la petición real | un middleware de UI veía **cero cabeceras** (§10.7) |
| — | `#[patch]`, `#[head]`, `RequestBuilder::form()` | métodos que faltaban, y un cuerpo de formulario a mano es donde se pierde el escapado |

Lo que era F4 —el edge— se quedó en la app, a propósito. El listener TLS
con resolver dinámico es genérico y subió como F2; el resto —qué host va
a qué contenedor, cuándo pedir un certificado, cómo se persisten las
rutas— está acoplado al modelo de despliegue. Un `wabot-feature-edge` con
traits `RouteTable`/`CertStore` sonaba limpio, pero sería una abstracción
diseñada con un solo consumidor.

### 2.2 Lo que explícitamente no hicimos

- **No un `wabot-feature-containerd`.** Es el producto.
- **No un segundo stack HTTP.** El edge es un `axum::Router` cuyo
  fallback despacha por `Host`.
- **No middleware que decore la respuesta.** Sigue siendo solo-rechazo;
  lo que haya que decorar va en un layer de tower.

---

## 3. El nodo: instalación y arranque

`wabot-deploy install` es una secuencia de pasos idempotentes. Cada uno
se registra en `node_state`, así que `doctor` puede contar la historia.

| # | Paso | Qué hace |
| --- | --- | --- |
| 1 | `preflight` | Linux, root, arch, systemd, cgroup v2 unified, overlayfs, :80 y :443 libres, disco |
| 2 | `layout` | `/etc/wabot-deploy/`, `/var/lib/wabot-deploy/{db,certs}`, modo 0700 |
| 3 | `config` | escribe `config.toml` si no existe |
| 4 | `database` | crea la BD y aplica migraciones |
| 5 | `runtime` | containerd, crun y los plugins CNI desde los tarballs oficiales, con checksum |
| 6 | `binary` | copia el ejecutable a `/usr/local/bin` si difiere |
| 7 | `service` | escribe la unidad, `daemon-reload`, `enable` |
| 8 | `certificate` | con dominio, orden ACME síncrona; sin dominio, CA local + hoja autofirmada |
| 9 | `start` | `systemctl restart`, que espera al `READY=1` |
| 10 | `report` | dónde está la consola y el token de setup |

**El ledger registra, no decide.** Es la lección que costó tres bugs
(§10.5, §10.8): cada paso es convergente por dentro —pregunta por la
cosa, no por la historia— así que saltárselo porque una ejecución
anterior dijo "hecho" solo produce respuestas viejas. El caso peor fue
un nodo que se quedó sin los plugins CNI porque `Step::Runtime` ya
estaba marcado de antes de que ese paso los incluyera.

### 3.1 Certificado o nada, pero después de arrancar

**El reto HTTP-01 lo contesta el nodo corriendo.** La respuesta se
guarda en la base de datos y se sirve en :80 desde `serve`, así que en
una máquina donde el nodo nunca ha arrancado no hay nada que conteste a
la autoridad y la orden solo puede terminar `Invalid`.

Por eso el certificado es el **último** paso, después del arranque. Y
por eso `install` no pide la orden él mismo: **mira**. El bucle de
renovación del nodo corre en cuanto arranca y instala lo que obtenga en
su resolver vivo; el instalador consulta la base cada dos segundos
durante minuto y medio y reporta. Pedirla por su cuenta —lo que hacía
antes, con un resolver propio— dejaba el certificado en la base y al
nodo sirviendo el viejo hasta su siguiente pasada.

Con eso, `install --domain X` **falla** si el certificado no llega:
código 1, la razón que el bucle dejó anotada, y el nodo **sigue
corriendo** y reintentando —la consola es la página donde alguien iría a
arreglar el dominio, y pararla sería quitársela—. Un install que reporta
éxito mientras sirve un certificado que ningún navegador acepta es un
fallo que se descubre después, en un navegador, con nadie delante.

Si nada arrancó el nodo —`--no-start`, `--no-system`, o una máquina sin
gestor de servicios— no falla: dice que el certificado se pedirá en el
primer arranque. Nada podía haber contestado el reto, así que culpar al
operador del orden de los pasos sería el error de antes al revés.

La salida sigue existiendo y hay que pedirla: `--allow-self-signed`. Una
máquina en red privada, o una cuyo DNS aún se propaga, es una forma real
de correr esto; lo que no puede ser es caer ahí por omisión.

### 3.2 crun, no runc

crun es C sin recolector de basura: **300 KB de binario frente a 15 MB**,
y arranca contenedores un 15–25 % más rápido. Con el API nativo de
containerd el runtime se elige **por contenedor**, no en la configuración
global:

```
Container.runtime = Runtime {
    name:    "io.containerd.runc.v2",       // el shim, no el runtime
    options: Any(containerd.runc.v1.Options {
        BinaryName:    "/usr/local/bin/crun",
        SystemdCgroup: true,
    }),
}
```

El shim `runc.v2` está mal llamado: es el shim para runtimes compatibles
con la CLI de runc, y `BinaryName` elige cuál. Consecuencia práctica: un
containerd preexistente —el que instaló Docker, por ejemplo— nos sirve
sin tocarle la configuración.

**`SystemdCgroup` sale de lo que corre la máquina**, no de una
constante. Con el driver de systemd el runtime le pide a systemd por
D-Bus que cree el slice del contenedor; en un nodo sin systemd eso es
imposible y crun falla con `cannot open sd-bus: No such file or
directory`, que no nombra ni a systemd ni al ajuste que lo pidió. Con
cgroupfs el runtime escribe la jerarquía él mismo. Los límites de
memoria y la contabilidad de OOM funcionan en los dos casos: lo que
decide el driver es *quién* crea el cgroup.

Ojo: `containerd-client` no vendoriza ese proto. Lo generamos nosotros
con prost.

### 3.3 Qué supervisa el nodo

systemd o **OpenRC** — Alpine es una máquina que este producto debería
querer: musl, un binario estático y un sistema base que deja el
presupuesto de RAM para lo que hace el trabajo. `bootstrap::init` sabe
dónde va un fichero de servicio, cómo se habilita, cómo se reinicia y
cómo preguntar si corre; **no** sabe qué dice ese fichero, porque una
unit y un script de init para el mismo servicio no comparten nada más
que la intención. Cada servicio trae sus dos textos.

Detección: `/run/systemd/system` primero —existe solo cuando systemd es
PID 1, a diferencia de `/usr/bin/systemctl`, que es solo un paquete—,
luego `/run/openrc`.

Sin ninguno de los dos, `install` escribe todo y **dice** que arrancarlo
es cosa tuya. Lo que no puede hacer es fingir que registró algo.

En OpenRC ambos servicios usan `supervise-daemon` con `respawn_max=0`,
que es la mitad de `Restart=always` que OpenRC no da por defecto. El
script del nodo declara `want containerd`, no `need`: un nodo cuyo
containerd gestiona otra cosa tiene que arrancar igual y decir qué pasa
en una página que alguien pueda leer, en vez de negarse a arrancar justo
lo que se lo iba a contar.

Y tres cosas que una distribución con systemd ya hizo por ti, y Alpine
no: montar la jerarquía de cgroups —`install` activa el servicio
`cgroups` de OpenRC, porque containerd lo declara como dependencia—,
cargar `overlay`, que se intenta con `modprobe` y se anota en
`/etc/modules`, y traer `iptables` e `iproute2`.

Los dos últimos son programas, no bibliotecas: el plugin bridge de CNI
enmascara con `iptables` y portmap escribe DNAT con él, y la creación
del netns por contenedor es `ip netns add`. Sin ellos todo instala bien
y el primer despliegue falla con `failed to locate iptables` en una
página que nadie asocia con la instalación. Así que `install` los pone
con el gestor de paquetes de la máquina —`apk`, `apt-get`, `dnf`— y, si
no hay ninguno, dice el comando exacto.

Es la excepción a "descargamos los tarballs oficiales": containerd, crun
y los plugins CNI llevan versión elegida por nosotros; iptables e
iproute2 son parte del sistema operativo, y llevar nuestra copia sería
pelearse con el kernel de la máquina.

La comprobación de `ip` le pide que haga el trabajo (`ip netns list`) en
vez de buscar el fichero: busybox trae un `ip` sin `netns`, y un chequeo
que solo buscara el nombre pasaría justo en las máquinas que fallan
después.

### 3.4 La unidad de systemd

Corre como **root**: necesita el socket de containerd, escribir en
`/var/lib/wabot-deploy` y enlazar :443. `Type=notify`, para que
`systemctl start` no vuelva hasta que el nodo sirve de verdad.

**Sin `PrivateTmp`, `ProtectHome` ni `ProtectSystem`**, y el motivo no es
pereza: ponen la unidad en su propio mount namespace, que systemd deja
*esclavo* del host — los montajes entran, no salen. El bind mount de
`/run/netns/<id>` se quedaba dentro y el shim de containerd, en otra
unidad, veía solo el fichero vacío que `ip netns` deja debajo. Un demonio
cuyo trabajo es construir namespaces y montajes para la máquina no puede
estar escondido del árbol de montajes de la máquina. Hay un test que
impide que vuelvan.

### 3.5 Composición de `serve`

```rust
ProjectRunner::new(container)
    .service_with_cancel("reconcile",  |c| reconcile_then_wait(deployer, c))
    .service_with_cancel("edge-https", |c| edge::serve_https(edge, resolver, c))
    .service_with_cancel("edge-http",  |c| edge::serve_http(port, db, c))
    .service_with_cancel("acme",       |c| acme::renewal_loop(db, config, resolver, wake, c))
    .run()
```

El primer servicio que termina tumba el proceso — que es lo correcto: un
nodo cuyo :443 murió no debe seguir pareciendo sano. De ahí que
`reconcile` espere la cancelación en vez de volver: reconciliar termina,
y un servicio que termina se lleva el nodo.

Antes de todo eso, `settle_after_restart` cierra la actualización que el
proceso anterior no pudo cerrar (§8).

---

## 4. El edge

```
                    :80                          :443 (rustls, vía F2)
                     │                                │
          ┌──────────┴──────────┐          ┌──────────┴───────────┐
          │ reto ACME http-01   │          │ SNI → nuestro resolver
          │ resto → 308 https   │          │      (tabla certificate)
          └─────────────────────┘          └──────────┬───────────┘
                                            despacho por header Host
                    ┌─────────────────┬───────────────┼────────────────┐
              consola + API      registry /v2      app del user     404
              (in-process)       (endpoints raw)   (proxy hyper
                                                    → 10.42.x.y:p)
```

- **Despacho por `Host`, no por SNI.** Con HTTP/2 una conexión TLS
  transporta peticiones de varios hostnames. El SNI solo elige
  certificado. Y se lee `:authority` *y* el header, porque HTTP/2 usa el
  primero y HTTP/1.1 el segundo.
- **La consola y el API no salen a la red.** El `Router` es un
  `tower::Service` y se invoca con `oneshot`. Cero sockets.
- **Tabla de rutas en `ArcSwap`**, hidratada al arrancar y reemplazada
  entera en cada cambio.
- **Sin rutas configuradas, todo llega al plano de control**, para que un
  nodo recién instalado sea alcanzable en su IP pelada.
- **`hyper::upgrade::on()` en las dos patas** del proxy, con
  `copy_bidirectional` entre ellas. Un proxy que solo reenvía cuerpos
  parece terminado y rompe todos los WebSockets.
- **Al contenedor se le habla HTTP/1.1**, aunque el navegador negocie h2
  con el edge: son dos negociaciones separadas, y eso es justo lo que
  permite terminar HTTP/2 delante de una app que nunca aprendió más.
- **308, no 302**, en el redirect a HTTPS: el método y el cuerpo tienen
  que sobrevivir.

### 4.1 Certificados

`instant-acme`, no `rustls-acme`: su lista de dominios se fija al
construir la configuración y no hay API para añadir uno en caliente.

**Regla dura: `ResolvesServerCert::resolve()` es síncrono.** No se emite
nada dentro del handshake. Se emite al enlazar el hostname, que es cuando
lo conocemos, y se renueva en background. Eso además cierra el DoS de la
emisión bajo demanda: quien mande SNIs aleatorios no puede quemarnos el
rate limit.

- **Un certificado por hostname**, no uno con muchos nombres: HTTP-01 no
  emite comodines, y un certificado multi-nombre habría que reemitirlo
  —y podría fallar entero— cada vez que un servicio cambia de dominio.
- **El emisor guardado es la URL del directorio**, comparada exacta. Un
  `starts_with("acme")` hacía pasar por bueno un certificado de staging
  (§10.6).
- **La caducidad se lee del certificado**, no se asume: Let's Encrypt
  emite a 90 días hoy y ha dicho que lo acortará.
- **El challenge vive en la BD**, no en memoria: una orden puede estar en
  vuelo cuando el nodo se reinicia.
- **La ruta del challenge se resuelve antes del fallback que redirige.**
  Un 308 mandaría a la autoridad contra un certificado que aún no existe.
- **`Wake`** es cómo el resto del nodo pide una pasada ya: la emisión
  pertenece al bucle, que es quien tiene reintentos y backoff. Sin él, un
  hostname nuevo quedaba enrutado y sin TLS durante las 12 h que el bucle
  duerme (§10.10).

### 4.2 El dominio del nodo es estado, no configuración

Vive en la tabla `setting`; el `config.toml` es solo la semilla. Lo
almacenado gana, porque una consola que un fichero puede sobreescribir es
una consola cuyos cambios dejan de aplicar en el siguiente reinicio.

La página del nodo lleva el emisor actual, el último fallo de ACME y un
formulario para fijar el dominio —el mismo u otro—, que **comprueba el
DNS antes de aceptar**. Pedirle a una autoridad que valide un nombre que
no apunta aquí gasta uno de los cinco intentos por hora para que te digan
lo que una consulta te decía gratis.

Volver a correr el instalador con otro dominio renombra el nodo de
verdad: certificado que cubre el nombre nuevo, ruta de control-plane
vieja retirada, nueva escrita —solo si ya había rutas— y reinicio aunque
el binario no cambie, porque el edge lee sus nombres al arrancar. Un
`install` sin `--domain` **no** pisa lo que se cambió desde la consola.

---

## 5. El modelo

```sql
project      (id, slug, name, network_index)
service      (id, project_id, slug, image, env, desired_state,
              last_error, address, current_release_id)
port         (id, service_id, container_port, host_port, hostname)
release      (id, service_id, digest, reference, created_at)
config_revision  (id, service_id, env, created_at)
account      (id, username, password_hash, node_role)
membership   (account_id, project_id, project_role)
invitation   (token_hash, node_role, project_id, project_role, used_at, expires_at)
route        (host, upstream_kind, upstream_addr, service_id)
certificate  (domain, names, cert_pem, key_pem, issuer, issued_at, not_after)
registry_tag (repository, tag, digest)
push_token   (id, project_id, name, secret_hash, last_used_at)
update_run   (id, from_version, to_version, tag, status, step, detail, backup_path)
setting      (key, value, updated_at)
```

### 5.1 Red por proyecto

Un bridge por proyecto (`wd-<n>`) y un `/24` dentro de `10.42.0.0/16`.
Contenedores del mismo proyecto se ven entre sí; de proyectos distintos
no, porque son dominios L2 separados. El índice se asigna en el primer
despliegue: hay 254, y un proyecto que no corre nada no debería gastar
uno.

Los plugins CNI se invocan directamente —el protocolo es "ejecuta este
binario con cinco variables de entorno y el config por stdin"— porque una
librería para eso es más dependencia que código.

**Un namespace de containerd, no uno por proyecto.** La separación es de
nombres y etiquetas. Un namespace por proyecto duplicaría el content
store, que es justo lo que el registry compartido existe para evitar. El
id de contenedor es `proyecto.servicio` — con punto, porque el regex de
containerd no acepta `--` y un punto no puede aparecer dentro de un slug,
así que el id se parte de vuelta sin ambigüedad.

### 5.2 Puertos

Una fila por puerto, dos columnas nullable. `container_port` es lo que el
proceso escucha dentro; `host_port` no nulo significa publicado en la IP
del nodo; `hostname` no nulo significa que responde HTTPS ahí. Las cuatro
combinaciones salen sin casos especiales, y la respuesta común a las dos
últimas es "no".

El hostname se verifica contra DNS **antes** de escribirse, comparando
contra el **dominio del nodo** y no contra su IP: el nodo no sabe de
forma fiable su dirección pública —detrás de NAT ve una privada— pero su
propio dominio ya resuelve a donde el mundo lo alcanza. Se intersecan los
conjuntos, así que un nombre con dos registros A sigue valiendo si alguno
coincide. El comodín se detecta sondeando una etiqueta aleatoria, porque
un registro comodín es invisible a una consulta directa.

**Las rutas se derivan, no se editan.** Son función de los puertos, los
servicios y sus direcciones, y el conjunto entero se recalcula tras cada
despliegue. La sincronización escribe siempre las filas del control
plane: el edge sirve la consola en todos los hostnames solo mientras la
tabla está vacía, así que la primera ruta de servicio habría dejado la
consola inalcanzable justo donde alguien iría a deshacerlo.

### 5.3 Estado observado, no recordado

El badge de un servicio sale de preguntarle a containerd, no de una
columna que alguien escribió al pulsar un botón. Un nodo que reporta lo
que le dijeron es un nodo que miente después del primer crash.
`Unknown` es un estado propio y distinto de `Not deployed`.

Reconciliar usa el mismo código que desplegar: por cada servicio que
debería estar corriendo, pregunta si lo está y despliega los que no.
Solo arranca cosas — un contenedor que ninguna fila reclama se deja en
paz y se reporta, porque borrar lo que el nodo no entiende es como un
reconciliador destruye datos.

### 5.4 Releases

**Una release es un digest, no una etiqueta.** Las etiquetas se mueven:
"vuelve al latest de ayer" no significa nada cuando el latest de ayer es
el de hoy. Y la actual está *marcada*, no derivada de "la más nueva",
porque un rollback hace que la actual sea una vieja.

**Dos historiales, a propósito.** Volver atrás una imagen y volver atrás
una configuración son intenciones distintas: lo normal es "esta build
está mal, corre la anterior, mantén los ajustes que arreglé".

### 5.5 Personas y permisos

Dos niveles porque hay dos clases de pregunta. "¿Puede crear proyectos,
invitar, mirar la memoria del nodo?" es del nodo: `admin` o `member`.
"¿Puede desplegar *aquí*?" es de un proyecto: `owner`, `deployer`,
`viewer`.

**Un administrador no es miembro de nada y llega a todo**: la pertenencia
responde la pregunta de proyecto y un admin nunca llega a esa pregunta.
Un nodo donde el operador tiene que añadirse a un proyecto para arreglarlo
es un nodo que lo deja fuera de lo que opera.

- **Un rol desconocido es el rol más pequeño.** Fallar hacia arriba una
  sola vez es una concesión silenciosa; hacia abajo es una negativa que
  alguien reporta.
- **Las decisiones son valores, no condiciones:** todo devuelve un
  `Access` y cada llamada pregunta por nombre (`may_deploy()`). La
  comparación escrita en treinta sitios es exactamente cómo una acaba sin
  una cláusula.
- **El filtro es la consulta, no la página.** Y "no es tuyo" responde
  idéntico a "no existe", o los nombres de todos los proyectos serían
  enumerables.
- **El invitado elige su propia contraseña.** El enlace lleva el derecho
  a crear una cuenta, una vez, antes de caducar. La cuenta se crea antes
  de gastar la invitación —un rechazo que el invitado puede arreglar no
  debe costarle el enlace— y gastarla es un `UPDATE … WHERE used_at IS
  NULL`.
- **No se puede** quitar al único owner de un proyecto ni degradar al
  único administrador del nodo: la vuelta atrás sería editar la base a
  mano.

El primer administrador se crea con un token que imprime `install`. La
alternativa —el primero que abra la web— es una carrera que pierde el
operador.

---

## 6. El registry

Los blobs entran directos al content store de containerd por su servicio
Content, y el manifest se convierte en un image record. Así una imagen
empujada **ya es** la imagen que el runtime corre: ni una segunda copia
en disco, ni un paso de importación. Ésa es la razón de escribir esto en
vez de correr un registry en un contenedor al lado, que guardaría cada
capa dos veces en un nodo cuyo diseño entero va de no hacer eso.

Cuatro cosas críticas:

1. **Registry y workloads en el mismo namespace de containerd**, o el
   contenido no es visible entre ellos y se pierde el ahorro.
2. **Lease sostenido durante todo el push.** Entre el commit del último
   blob y el `Images.Create` los blobs no tienen referencias y un pase de
   GC se los lleva.
3. **Labels de GC con las convenciones de containerd**
   (`containerd.io/gc.ref.content.*`). Inventar las nuestras rompe `ctr`,
   `nerdctl` y el propio GC.
4. **Escribir por gRPC, leer del filesystem.** Los blobs son inmutables y
   direccionables por contenido; servir un GET a través del stream de
   protobuf es caro.

La sesión de subida es de containerd, no nuestra: su escritura de
contenido es una transacción con nombre, reanudable por ref y offset. Así
que el id de la subida **es** el ref y el módulo no guarda estado entre
peticiones.

**El índice de etiquetas es nuestro** (`registry_tag`), no el de
containerd: compartir el *contenido* es el diseño, compartir el *espacio
de nombres de etiquetas* no lo era, y un `ctr images tag` hacía que el
registry dijera "ya lo tengo" de imágenes que nadie había empujado.

**Un push deja blobs, no snapshots.** El servicio Transfer desempaqueta
como parte de un pull; pedirle transferir una imagen del store a sí misma
reporta éxito y no desempaqueta nada (probado en el nodo, dos veces). Así
que lo hacemos como lo hace un unpacker: por cada capa, preparar un
snapshot sobre la cadena, aplicar el diff y comprometerlo bajo el chain
ID que el runtime buscará.

Autenticación: **token de push por proyecto**, aceptado como contraseña
Basic, que es lo que `docker login` y `ctr push` saben mandar.

---

## 7. La consola

Server-rendered con `rsx!` de hypertext. Sin JavaScript, sin build step,
sin payload de cliente — la forma correcta para algo que tiene que
funcionar en una caja de menos de un giga.

**hypertext y no Maud** por una razón: valida nombres de elemento *y* de
atributo en tiempo de compilación. Maud compila `<dvi klass="x">` y emite
el HTML malformado.

- **Formularios planos: POST y 303.** Funciona con el scripting apagado,
  y la defensa CSRF es `SameSite=Lax` en la cookie en vez de un token que
  enhebrar por cada formulario. Los errores vuelven en un parámetro de
  query: un formulario rechazado no es secreto, y así no hay estado con
  tiempo de vida.
- **El middleware anota, no rechaza.** Un rechazo del framework es un
  cuerpo JSON, y para un navegador la respuesta correcta es la página de
  login. Así que el middleware siempre tiene éxito —lee la cookie y, si
  nombra una sesión viva, asigna la cuenta a `Auth`— y cada vista decide.
  Consecuencia: **el POST es la frontera, no la página**; cada endpoint
  que muta vuelve a comprobar, y hay un test por endpoint que lo fija.
- **El marco lleva datos propios, no prestados.** `Frame` copia usuario,
  proyectos y ruta: el marco nombra el proyecto actual mientras la página
  dibuja ese mismo proyecto, y el cierre que construye `rsx!` captura por
  movimiento.
- **La URL es la selección de proyecto**, no una preferencia guardada, y
  el selector es un formulario con botón en vez de un `<select>` que
  navega.
- **Un nodo, en plural.** Hay uno y se lista como una lista de uno: la
  forma de la página es la que recibiría un segundo, y una lista que
  empieza siendo página de detalle no se convierte en lista sin romper
  todos los enlaces.

### 7.1 La memoria, atribuida en vez de sumada

"Usada" es un número que muestra cualquier herramienta y sobre el que
nadie puede actuar. Lo que necesita quien opera este nodo es qué parte es
la plataforma y qué parte es lo que desplegó. Cinco líneas:
wabot-deploy, containerd, los shims, los contenedores y todo lo demás.

Los shims van aparte: existen *porque* hay un contenedor, pero son coste
del runtime, no de la imagen.

**Las partes no cuadran, y decirlo es el punto.** El `memory.current` de
un contenedor incluye su page cache, que también cuenta en el `Cached`
del sistema; el RSS de dos procesos cuenta dos veces las páginas
compartidas. "Todo lo demás" es un resto, no una medida. La alternativa
es un número que cuadra exacto y no significa nada. La página lo dice en
voz alta debajo de la tabla.

Se lee de `/proc` y del árbol de cgroups, sin crate intermedio, y el
cgroup de un contenedor se encuentra por el pid de su tarea, no
adivinando la ruta.

**El único JavaScript del producto** es un `EventSource` que sustituye
esas cifras cada dos segundos. La página renderiza completa y correcta
sin él, así que la consola sigue sirviendo en la máquina donde el stream
no se puede abrir — que es justo la máquina desde la que alguien mira
esta página.

---

## 8. Actualizaciones

El nodo se trae una release publicada en GitHub **cuando alguien se lo
pide**. Nada de esto corre por temporizador: un nodo que se actualiza
solo es un nodo que reinicia todo lo que lleva encima en un momento que
nadie eligió.

**El orden de los pasos es la seguridad:**

1. Descargar el `.sha256` y el binario, y compararlos.
2. Ejecutar lo descargado con `--version` y exigir que diga lo que el tag
   promete. Esto caza dos cosas que el checksum no ve: un binario para
   otra arquitectura o libc, y un release cuyo asset no coincide con su
   tag.
3. Copiar la base con `VACUUM INTO` —no copiando el fichero: SQLite se
   está escribiendo mientras tanto— porque la migración es el único paso
   que devolver el binario viejo no deshace.
4. `rename` atómico, dejando el anterior como `.previous`.
5. Reiniciar.

**El reinicio se hace desde fuera del cgroup.** Un `systemctl restart`
lanzado desde dentro de la unidad se mata a sí mismo al parar la unidad;
`systemd-run --on-active=1` crea una unidad transitoria que sobrevive. En
OpenRC no hay tal cgroup, pero el comando muere igual con la sesión de su
padre, así que se le da una propia con `setsid`.

**Quién informa del resultado.** El último paso reemplaza el proceso, así
que la fila queda en `restarting` y la resuelve el nodo que vuelve,
comparando su propia versión con la que la fila decía. Por eso ese estado
vive en la base y no en memoria: la página que el navegador recarga
después la sirve el binario *nuevo* leyendo lo que escribió el viejo. Si
vuelve en otra versión, la página dice *Failed* con la versión real en
vez de un éxito falso.

**Las notas de cada release** se leen en la consola. Vienen en Markdown y
se parsean a *estructura*, no a HTML: `rsx!` escapa cada valor, así que
una nota que contenga `<script>` es un párrafo que dice `<script>`. Solo
los enlaces `http(s)` son enlaces. El catálogo se cachea quince minutos
—GitHub permite sesenta peticiones sin autenticar por hora y por IP— con
un botón para preguntar otra vez.

**Lo que todavía no hace:** no reescribe la unidad de systemd, así que un
release que la cambie tiene que decir en sus notas que hay que volver a
correr `install`. Y no hay botón de rollback: volver atrás una migración
no es una operación de ficheros, que es para lo que está la copia.

---

## 9. Dependencias y presupuesto

### 9.1 Lo que hay dentro

```toml
wabot = { version = "0.2", default-features = false, features = [
    "rest", "rest-tls", "sqlite", "ui-hypertext", "tracing-format",
] }

hyper / hyper-util / http-body-util    # el proxy y el cliente de updates
hyper-rustls                            # raíces empaquetadas, ring
instant-acme                            # ACME, default-features = false + ring
rcgen + x509-parser                     # CA local y hojas autofirmadas
containerd-client + tonic + prost       # el API nativo de containerd
oci-spec                                # specs OCI tipadas
arc-swap                                # la tabla de rutas
hypertext                               # el renderizador de la consola
rusqlite (vía el framework, bundled)    # SQLite dentro del binario
sha2, base64, toml, serde, clap, tokio
```

`default-features = false` en el framework es obligatorio: el default
arrastra todo el stack LLM.

**Un solo backend criptográfico.** `instant-acme` trae `aws-lc-rs` por
defecto y el framework fija `ring`; con los dos activos rustls se niega a
elegir y **panica en el primer uso** — o sea, en un nodo real, no solo en
tests. Además aws-lc-rs necesita cmake, lo que rompería la propiedad de
"compila en una máquina pelada".

`tokio` con features explícitas, no `"full"`, y `worker_threads` acotado:
el default es un worker por core, y en un nodo de 16 son 16 stacks para
un plano de control que casi no hace CPU.

### 9.2 Dónde está de verdad la RAM

| Componente | Coste | ¿Lo cambia crun? |
| --- | --- | --- |
| `containerd-shim-runc-v2`, uno por contenedor | **~11 MB RSS, persistente** | **No** |
| El proceso del runtime (`crun`/`runc`) | efímero | Sí — 300 KB vs 15 MB, y 15–25 % menos latencia |
| `wabot-deploy` | 13 MB medidos | — |
| `containerd` | ~20 MB | — |

crun gana en tamaño de binario, en latencia de arranque y en la memoria
pico de cada `create`, **no** en el residente por contenedor. El suelo
persistente es el shim, y es Go. Si algún día estorba, el shim `runc.v2`
agrupa contenedores en un solo proceso según labels — es lo que hace CRI
para meter todos los contenedores de un pod en un shim. No hace falta con
una réplica por servicio.

### 9.3 Cómo se construye y se despliega

- `cargo build --profile node` — LTO fino, cuatro unidades de codegen. Es
  lo que usa `scripts/deploy.sh`, que compila **en el nodo** por SSH: un
  build de `release` en una VM de un núcleo se lleva la máquina por
  delante, rustc tira de swap hasta que sshd deja de contestar.
- `cargo build --release` — LTO completo, una unidad, sin símbolos. Es lo
  que construye CI para publicar.
- **Un tag es el disparador, y el tag es la versión.** El workflow
  comprueba que el tag y el `Cargo.toml` coincidan antes de construir.
- **musl estático**: un build con glibc lleva como suelo la versión del
  runner, y las máquinas donde esto aterriza no siempre son más nuevas.
  El coste es un asignador más lento bajo mucha concurrencia; este nodo
  hace de proxy y escribe SQLite, no es esa carga.

---

## 10. Registro de decisiones

Un apartado por hito: qué se decidió, y dónde la realidad contradijo el
plan. Las lecciones que se repiten están arriba, en su sección.

### 10.1 A1 — cancelación, systemd, TLS (framework)

`Cancel` es un canal `watch`, no un flag: un flag hay que sondearlo y un
listener parado en `accept()` nunca llega a sondearlo. **Latched, no un
flanco**, para que un servicio que arranca durante un apagado en curso no
espere una segunda cancelación que no llegará.

**El runner no esperaba a sus servicios**: la rama de señal del `select!`
soltaba el `select_all` entero y con él todos los futures. Ese era el bug
de fondo. Ahora van a un `JoinSet` y se drenan bajo timeout.

En TLS: ALPN lo pone el framework —olvidarlo es invisible, todo funciona
pero todos los clientes negocian HTTP/1.1—, y
`serve_connection_with_upgrades`, sin lo cual todo WebSocket sobre TLS
falla y *solo* sobre TLS.

### 10.2 A2 — SQLite (framework)

`created_at` es INTEGER de epoch-millis: SQLite no tiene tipo fecha y la
paginación keyset compara esa columna constantemente.

`PRAGMA case_sensitive_like=ON` no es cosmético: el `LIKE` de SQLite es
insensible por defecto y sensible en todo lo demás donde corre el
framework, así que el mismo `Query` significaría cosas distintas según el
backend.

**Hallazgo colateral que valía el desvío:** la feature `macros` de sqlx
arrastra `sqlx-sqlite`, así que **todas las builds solo-Postgres venían
compilando un driver de SQLite** que nadie usaba.

### 10.3 A3 — cuerpos crudos (framework)

`#[raw]` es un atributo hermano, no un flag dentro de `#[get(...)]`: así
la gramática de la ruta sigue siendo "una ruta, o nada". Lo que importa
del wrapper es el **orden**: parte la petición, corre los middlewares
sobre las `Parts` y la reensambla — de ahí que los guards sigan
corriendo y los extractores de axum funcionen dentro del handler.

Dos errores de compilación deliberados, verificados compilándolos:
`#[raw]` + `#[max_body]` juntos (un endpoint crudo no bufferiza, así que
un límite es una creencia falsa sobre el código), y cualquiera de los dos
sin atributo de ruta (compilarían en silencio y no se montarían nunca).

### 10.4 M0 — el esqueleto

Binario de 4,2 MB y 7 MB de RSS. Cuatro decisiones que no estaban en el
plan: el plano de control escuchaba en loopback mientras no hubo
autenticación; `deny_unknown_fields` en todo el `config.toml` —una errata
deja al operador creyendo que cambió algo—; `/healthz` y `/readyz`
separados; y el ledger con los nueve pasos declarados desde el principio,
listando como `not implemented yet` los que faltaban en vez de
esconderlos.

Se quitaron dos cosas por especulativas —una tabla `setting` sin usuario
y un `open_in_memory` público— porque escribir el schema completo de una
vez es escribir código sin usuario.

### 10.5 M1 — el edge

**La simplificación que cambió el diseño:** no hace falta escribir el
accept loop a mano. F2 ya termina TLS con resolver dinámico, maneja
upgrades y drena, así que el edge es un `axum::Router` cuyo fallback
despacha por `Host`.

**Un bug real que cazó un test:** las firmas ECDSA son aleatorizadas, así
que reconstruir la CA con `from_ca_cert_pem().self_signed()` **re-firma**
y produce bytes distintos en cada arranque. El fingerprint que el
operador confía habría cambiado en cada reinicio.

### 10.6 M2 — ACME

**El bug del tipo peor, y solo apareció en el nodo real:** probé contra
staging, cambié a producción, reinicié — y el nodo siguió sirviendo el
certificado de staging. Sin error, sin aviso. `issuer.starts_with("acme")`
matcheaba también `acme-staging`. La corrección es guardar la URL del
directorio y comparar exacto. El test de regresión se validó
**reintroduciendo el bug**.

También: los tests de instalación empezaron a pedir certificados de
verdad para un dominio que no es nuestro, contra producción. Ahora
`config_in` pone `acme.disabled = true`, con el porqué al lado.

### 10.7 M4 — cuentas, proyectos y servicios

**Un bug del framework, del tipo silencioso:** con la sesión creada y la
cookie correcta, la consola seguía mandando a `/sign-in`. El macro
`#[view]` le pasaba a `produce` un head *sintético* reconstruido desde la
URI, así que **cualquier middleware sobre una vista veía una petición sin
cabeceras** — y como los middlewares son solo-rechazo, la forma de no
encontrar nada era dejar pasar a todo el mundo. `Parts` implementa
`Clone`; la copia sintética no hacía falta.

Dos cosas que solo aparecieron en el nodo: `install` dejaba corriendo el
binario viejo —el paso `Start` estaba en el ledger— y `install` quemaba
un token de setup todavía válido, o sea convergía rompiendo justo lo que
te acababa de dar.

Del dominio: la contraseña no se recorta —los espacios son parte de ella—;
usuario desconocido y contraseña mala responden igual y pagan el mismo
hash; el env se parte por el *primer* `=`, porque los valores que alguien
pega son URLs de conexión y blobs base64.

### 10.8 M5 — los contenedores corren

**Por qué la red llegó antes de lo planeado:** con el netns del host
compartido, `container_port` no significa nada y `nginx:alpine` choca con
el edge. Un contenedor que hay que modificar para poder desplegarlo no es
una plataforma de despliegue.

Cuatro fallos, los cuatro en el nodo: el ledger por tercera vez (§3);
`project--service` rechazado por containerd (§5.1); `setns: Invalid
argument`, que no nombra ni a systemd ni a la propagación (§3.3); y DNS
que funcionaba para `wget` y no para `nslookup`, porque el host daba
cuatro resolvers y los dos primeros eran IPv6 sobre un bridge IPv4.

### 10.9 M6 — puertos y HTTPS por servicio

Tres fallos, los tres en el nodo: el proxy hablaba HTTP/2 a un contenedor
que solo sabe HTTP/1.1 (`UserUnsupportedVersion`, que no nombra a ninguno
de los dos extremos); `portmap DEL` con lista vacía no borra nada, y el
caso que más necesita limpieza es justo el que la deja vacía; y el puerto
de relleno era `0`, que el plugin rechaza al parsear.

**Límite conocido:** reconciliar solo mira si el contenedor corre, no si
sus mappings coinciden con las filas.

### 10.10 M7 — la consola con marco

Un fallo que solo aparece al cambiar un servicio en caliente: dar un
hostname creaba la ruta y **nadie pedía el certificado**, así que el
nombre quedaba enrutado y sin TLS durante las 12 h que el bucle duerme.
De ahí `Wake` (§4.1). Y la página dice "Certificate on the way" mientras
tanto: se pregunta, no se supone.

### 10.11 M8 — personas y permisos

Ver §5.5, que es donde vive el modelo. Lo que se verificó en el nodo con
una invitación real: el miembro solo ve su proyecto, `second-project`
responde "no such project", `/people` y `/nodes` redirigen, borrar el
proyecto se rechaza por rol, desplegar se permite, y el enlace usado
dice "not valid".

### 10.12 M9 — registry, releases y rollback

Cinco fallos, y ninguno se veía sin el nodo: el índice de etiquetas
compartido con containerd (§6); la referencia de la release sin host, que
containerd leía como un registry a resolver por DNS; desplegar por digest
sin registro local; el push que deja blobs y no snapshots (§6); y
**cambiar una variable movía el servicio de imagen**, porque el
redespliegue usaba la etiqueta del servicio en vez de la release marcada
— justo lo que las releases existen para evitar. Lo mismo al reconciliar.

Además, dos cosas del protocolo: el PUT final de un blob no lleva
`Content-Range`, así que el total hay que preguntárselo a
`Content.Status` o el cliente reintenta cada capa; y una sesión gRPC por
PATCH con trozos de 1 MB, contra los timeouts de TLS.

### 10.13 M10 — el framework publicado y el pipeline

`wabot 0.2.0` en crates.io, así que desaparece el `[patch.crates-io]`.
Eso arregló de paso el CI, que llevaba roto sin que se notara: el patch
apuntaba a una ruta que en un runner no existe, y además faltaba
`protoc`.

La verificación en hardware real cazó un fallo en el propio workflow: la
comprobación de "¿es estático?" buscaba la frase `not a dynamic
executable` y el `ldd` de esa máquina dice `statically linked`. Habría
rechazado exactamente el binario que debía aceptar. Ahora busca
bibliotecas resueltas (`=>`), que es el hecho y no la redacción.

### 10.14 M11 — el dominio como estado

Ver §3.1 y §4.2. La raíz común de las dos correcciones: el nodo tenía
hechos que solo su configuración de arranque podía cambiar. Mover el
dominio a `setting` obligó a repasar cada lector —el arranque del edge,
el bucle de renovación, las rutas, la sugerencia de hostname, la lista de
nodos y `doctor`— y a mudar el último error de ACME, que estaba en la
fila del certificado: justo la que no existe cuando el fallo es "pedí un
nombre y no obtuve nada".

### 10.15 M12 — actualizarse con un clic

Ver §8. Verificado de punta a punta en el nodo: 0.1.0 → 0.1.1 desde la
consola, con `update settled status="done"` escrito por el binario nuevo
leyendo la fila del viejo, la copia de la base en `backups/`, el binario
anterior en `.previous` y los dos contenedores intactos.

### 10.16 M13 — Alpine

Un intento de instalar en Alpine se paró donde tenía que pararse —al
arrancar containerd— y dejó ver que el nodo confundía "systemd" con
"algo que supervisa servicios". Ahora esa pregunta tiene un tipo
(`bootstrap::init::Init`) y tres respuestas, y todo lo que la hacía
—`install`, `doctor`, el preflight, el actualizador— pregunta por lo que
le importa: *¿hay algo que mantenga esto vivo?*

Lo que Alpine enseñó y una distribución con systemd escondía: la
jerarquía de cgroups no se monta sola, `overlay` no se carga solo, y
`Restart=always` no es gratis en OpenRC —hay que pedir
`supervise-daemon`—. Nada de eso es exótico; es lo que systemd venía
haciendo sin que nadie lo escribiera.

Y luego dos que sí eran de Alpine, encontradas desplegando de verdad:
`iptables` no viene en el sistema base —el plugin bridge lo necesita— y
`SystemdCgroup = true` era una constante en un producto que ya sabía
que systemd no siempre está.

Y una tercera, que no era de Alpine en absoluto: **la primera
instalación con dominio no podía funcionar en ninguna máquina**. Pedir
el certificado antes de arrancar el nodo es pedirle a la autoridad que
valide un reto que nadie está sirviendo; como desde 0.1.1 eso era fatal,
el nodo no arrancaba nunca y la siguiente ejecución fallaba igual. En el
nodo Ubuntu no se vio porque el servicio ya llevaba corriendo desde
instalaciones anteriores: la orden se validaba contra el nodo que ya
estaba en marcha. Ver §3.1.
