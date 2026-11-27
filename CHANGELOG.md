# Registro de cambios — Aura Sentinel

Todos los cambios relevantes del proyecto se documentan aquí. Formato: `[versión] — fecha`, con
[versionado semántico](https://semver.org/lang/es/).

---

## [4.13.21] — 2026-11-27 (un proyecto grande ya no bloquea a Aura, el agente no puede falsificar su revisión y un merge no pierde tu trabajo)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10. Sale de una auditoría real de un repositorio grande y de la batería de 12 misiones con Qwen3.6.

- **Un proyecto grande ya no bloquea a Aura.** En una auditoría de un repositorio con 1096 archivos `.py`, cada orden respondía «Error interno: FileNotFoundError: [WinError 206]», también tras cerrar la app o abrir otra ventana:
  al empezar cada tarea se pasaban todas las rutas en la línea de comandos y Windows corta en 32.767 caracteres. Ahora, si no caben, van en un archivo; y si un analizador no arranca o tarda demasiado, queda un aviso y la tarea sigue.
- **El agente ya no puede tocar su propia revisión.** En una misión, tras una revisión que pedía cambios, editó `.agente/revision.json` y vació el registro hasta que `terminar` pasó con «Revisión: APROBADO».
  Ahora no puede escribir, borrar ni mover los archivos de estado de `.agente/`, y si la revisión cambia por fuera vale la real y queda anotado.
- **Un merge no pierde lo que la otra rama no tocó.** Al resolver un conflicto se quedaba con un lado entero y borraba una línea que ya estaba en `main`. Ahora las marcas de conflicto enseñan también la base, y el commit del merge se rechaza (hasta 2 veces) si quedan marcas o faltan líneas que solo un lado cambió, también si el commit se hace con una orden normal.
- **El revisor ve lo hecho con git.** Antes podía pedir «pruebas» de un merge que sí se había hecho.
- **Un `.bat` que no encuentra su proyecto ya no pasa.** `verificar` y `crear_bat` comprueban que `Cargo.toml`, `go.mod`, `package.json`, etc. estén en la carpeta donde corre la orden del doble clic.
- Fuera del instalador: banco de 12 misiones y radar de novedades en `bench/`.
- Medido: 1006 pruebas pasan, 3 omitidas (130 nuevas); ruff limpio. Con Qwen3.6, el motor sin el arreglo de proyectos grandes dio 10 de 12 misiones correctas según el juez automático (11 al revisarlas a mano).
  **No medido:** el arreglo de proyectos grandes no se ha probado con el modelo real (sí en una copia del repositorio que fallaba, con un modelo de mentira). En repositorios ajenos, `verificar` todavía puede marcar errores que no lo son e intentar ejecutar su código.

---

## [4.13.20] — 2026-11-24 (hasta 200 pasos, `editar` ayuda más cuando falla y una verificación que se cae ya no tira la misión entera)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10. Sale de las ferreterías y veterinarias de las últimas tandas con Qwen3.6 (algunas llegaban al tope de pasos o se caían) y de una ferretería con Gemma que se detenía al editar.

- **Hasta 200 pasos en vez de 150.** Entre las 123 carpetas de pruebas guardadas el percentil 90 era de 151 pasos y el máximo de 200: varias entregas se cortaban en el tope con el plan casi hecho. Es un tope, no una meta: las dos ferreterías de la tanda siguiente terminaron igualmente a los 96-97 pasos.
- **`editar` ayuda cuando no acierta.** En 8 misiones hubo 17 `editar` que no encontraban el texto, casi todos seguidos de un `leer` del mismo archivo. Ahora, si no lo encuentra, enseña las líneas más parecidas con su número para copiarlas;
  si `buscar` aparece varias veces, da para cada aparición la línea anterior que la distingue; si el `buscar` trae otra sangría que el archivo, reindenta el reemplazo (un `buscar` de 8 espacios sobre código a 12 dejaba el bloque descolocado y con error de sintaxis);
  si el `.py` queda roto, dice en qué línea y enseña el trozo; del 3.er intento fallido seguido sobre un archivo recuerda la salida (un `buscar` de una sola línea, o `escribir`);
  y un reemplazo que deja el archivo exactamente igual ya no se da por «OK editado»: dice «Sin cambios» y no escribe (una pasada con Gemma repitió 16 ediciones así hasta que el bucle la cortó).
- **Una verificación que se cae ya no tira la misión.** En una veterinaria, `terminar` lanzó la verificación automática, el recorrido web siguió un enlace de la propia app con un espacio sin codificar (`/admin/clientes?q=Juan Perez`) y el programa entero se cerró a los 137 pasos con un error sin mensaje.
  Ahora `terminar` y el rescate que se hace al llegar al tope de pasos atrapan cualquier error de la verificación (el fin se rechaza con el motivo), y el recorrido codifica los espacios y las letras no ASCII de las direcciones que sigue.
- **Aviso si la revisión ya aprobó y no se termina.** Dos veterinarias llegaron a los 200 pasos con la revisión aprobada sin llamar nunca a `terminar`. Ahora, a los 8 pasos de una revisión aprobada sin terminar, el agente recibe el aviso de que, si no falta nada del plan, llame a `terminar`.
- **Un empujón si el primer turno es solo texto.** Con Gemma, un encargo largo acabó en el primer turno con el plan escrito en el chat y ninguna herramienta llamada. Ahora, si el modelo contesta solo con texto a un encargo largo sin haber llamado a ninguna herramienta, se le empuja una vez a empezar con ellas (en las 129 misiones guardadas de otros modelos no habría saltado nunca).
- Medido sin el modelo: 55 pruebas nuevas (876 pasan, 3 omitidas); las de `editar` y las del recorrido web fallan sin el cambio y pasan con él; ruff limpio. Con Qwen3.6, tras subir el tope y añadir el aviso: de dos veterinarias, una terminó (166 pasos; llamó a `terminar` 22 s después de la revisión aprobada, sin que hiciera falta el aviso)
  y la otra llegó a los 200 pasos arreglando sus propios tests. **No medido:** los cambios de `editar` con Qwen3.6, el aviso de revisión aprobada en una misión real (no llegó a darse el caso), ni la caída de `terminar` (el error no volvió a aparecer; solo lo prueban las pruebas del motor). Con Gemma 26B (la variante abliterated instalada) la ferretería no se terminó en ninguna de 7 pasadas; los avisos nuevos se disparan pero Gemma no los obedece.

---

## [4.13.19] — 2026-11-22 (el agente ya no se atasca repitiendo `leer` y `pip install paquete>=3.1` sin comillas deja de perder la salida y la versión)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10. Sale de la ferretería (23), la repetición del mandato con la 4.13.18: se detuvo a los 106 pasos (plan 7/10, sin ningún `verificar`) y dejó un archivo suelto «3.1».

- **`leer` sigue donde se quedó.** La misión repitió `leer(lineas=340)` sobre `app.py` (501 líneas) y el bucle la cortó con «el modelo repitió 'leer' 5 veces sin avanzar»:
  tras leer las líneas 1-340, la misma llamada recibía un mensaje («ya leíste este archivo… usa desde=341») sin contenido y el modelo (Qwen3.6) no lo leía. 36 de las 106
  llamadas fueron `leer`, con rachas parecidas tras cada compactación del contexto (pasos 44, 69 y 90). Entre las 24 carpetas de pruebas con chat hubo 7 paradas por repetición
  en 5 carpetas, 3 de ellas de `leer`. Ahora, con el principio ya leído y sin `desde`, `leer(lineas=N)` devuelve las líneas siguientes con una nota de una línea (y alarga lo leído;
  un `desde` que sigue justo donde se quedó también); si el archivo ya está entero lo dice sin contenido. Y al compactar el contexto se olvida lo leído: el modelo ya no ve
  ese contenido y antes se le negaba volver a pedirlo. Repetidas sobre una copia las llamadas reales de la (23) y de otra carpeta de pruebas: `app.py` pasa de 336 a 501 líneas
  servidas de 501 y `gui.py` de 75 a 700 de 700.
- **`pip install paquete>=3.1` sin comillas.** En cmd el `>` sin comillas dobles es una redirección: en la (23) la salida de pip fue a un archivo «3.1» (el modelo vio `exit=0` sin
  nada más) y el paquete se instaló sin la versión pedida; las comillas simples no protegen. En las 145 carpetas guardadas había 11 archivos así en 9 carpetas, y `verificar` nunca los vio.
  Ahora `ejecutar` rechaza ese comando sin ejecutarlo y da el comando corregido; `crear_bat` no crea un `.bat` con esa línea; y `verificar` avisa (nunca error) de la línea de un `.bat`
  que ya lo tenga y del archivo suelto con nombre de versión y salida de pip. No se rechaza `2>&1`, `> log.txt`, `-r requirements.txt`, `pip freeze > req.txt` ni un requisito entre
  comillas dobles; vale para `pip install`, `uv pip install` y `uv add`.
- Medido sin el modelo: las llamadas reales de las dos carpetas (arriba), los 11 archivos y los 11 comandos reales (9 carpetas) detectados sin ningún falso en 145 carpetas ni en sus
  222 `.bat`, y el tiempo no cambia. **No medido:** con el modelo, si deja de repetir la llamada o corrige el comando tras el rechazo (el corte a la quinta repetición sigue ahí).
  821 pruebas del motor pasan (22 nuevas; 4 omitidas); ruff limpio en el motor y en los tests tocados. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.18] — 2026-11-22 (`verificar` ve dos fallos más de una web —ventas de otros en una tabla sin vendedor y librerías cargadas de internet— y el agente tiene una guía de diseño visual)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10. Sale de la ferretería (22), la repetición del mandato con la 4.13.17: la entrega salió bien en lo que `verificar` comprueba y aun así tenía dos fallos que no veía.

- **Ventas de otros en una tabla sin columna de vendedor.** `/ventas/nueva` enseñaba al empleado las 10 ventas más recientes de todos (total y cliente),
  sin filtrar por vendedor, y `verificar` dio 0 avisos: la tabla no tiene columna de vendedor (la regla del panel de la 4.13.17 no la cogía) ni enlaces de detalle
  (la de «filas» tampoco). Ahora, en la segunda lectura del empleado, una tabla con importe y folio/factura/venta (o cliente junto a fecha, hora o producto), sin
  columna de vendedor y con alguna fila de importe distinto de cero da un aviso (nunca un error) con la salida «ignora este aviso»: el empleado de la prueba no ha
  vendido nada, así que una lista filtrada por vendedor sale vacía. No cuenta listas de productos o de clientes con un «Total comprado», tablas vacías, «No hay
  ventas», filas de totales ni una ruta ya avisada por otra regla. Sin peticiones nuevas.
- **Librerías cargadas de internet.** La (22) cargaba Bootstrap 5.3.3 de `cdn.jsdelivr.net` y el mandato era «que funcione en mi computadora con un doble clic»: sin red
  la web pierde el estilo y los menús, y `verificar` la abre con red y no lo nota. Ahora, en las webs con servidor, un aviso nombra los `<link>` o `<script src>`
  con `https://` (o `//`) y los `@import` de un CSS (Bootstrap, jQuery, Tailwind, Chart.js, tipografías), con su plantilla, y da las dos salidas: copiarlos a `static/`
  y enlazarlos con `url_for('static', ...)`, o ignorar el aviso si la web necesita internet (también se calla si el README de la raíz lo dice). No cuenta iconos,
  enlaces `<a>`, imágenes, comentarios, `localhost`, tests ni dependencias, ni páginas o juegos estáticos (los de Three.js por importmap).
- **Guía de diseño visual (`guia("diseno")`).** El agente no tenía nada sobre aspecto: la guía `web` traía una línea de CSS. En las 104 webs guardadas con HTML lo básico
  ya estaba (viewport 100 %, `@media` 77 %, variables CSS 75 %), pero casi todas repetían la misma paleta (`#2563eb` en 74, `#dc2626` en 73) y 67 usaban emojis como iconos.
  La guía (3,2 KB) pide el color de marca según el negocio, la paleta en `:root`, un solo CSS sin CDN, sin emojis como iconos, componentes iguales, estados (foco,
  vacío, error), móvil de 375 px sin scroll horizontal y accesibilidad; lo que pida el usuario manda. Se encuentra por «diseño», «ux», «ui», «estilo», «responsive»…,
  el método la nombra en la lista de guías con una regla de una línea y `web.md` remite a ella.
- Medido sin el modelo: la (22) da los dos avisos nuevos y las copias corregidas (Bootstrap en `static/`, lista filtrada por vendedor) ninguno; en 123 carpetas
  guardadas, 6 avisos de tablas de ventas (5 webs) y 13 de librerías de internet (las 13 que las cargan), todos vistos a mano y verdaderos, 0 falsos; el tiempo por web no cambia
  (13,2 s). **No medido:** la guía de diseño con el modelo (no se sabe si el agente la lee ni si cuesta más pasos). 799 pruebas del motor pasan (37 nuevas; 4 omitidas);
  ruff limpio en el motor y en los tests tocados. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.17] — 2026-11-22 (`verificar` ve tres fallos más de una web: contraseña que solo existe en los tests, panel del empleado con ventas ajenas y campo de formulario que no cuadra)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10. Sale de dos pruebas reales (la ferretería y la veterinaria) en las que `verificar` dio «todo bien» y no era cierto.

- **Contraseña del resumen que solo existe en los tests.** El mensaje final daba `empleado/empleado123`, pero `empleado123` solo aparecía en `test_app.py`
  y el programa solo creaba `admin`. Ahora `verificar` rechaza una contraseña del resumen que solo está en archivos de tests y explica las dos
  salidas (que el programa la cree, o quitarla del resumen). Cuenta como existente si está en el código, en un `seed`, en el README, en un `.env`
  o en un `config.json`.
- **Panel de inicio del empleado con ventas de otro vendedor.** El empleado entraba en `/` y veía «Últimas ventas» hechas por el administrador. Ahora,
  tras el recorrido, `verificar` mira cada página que abre el empleado y, si una tabla de ventas trae una columna de vendedor con otra persona
  (vendedor, vendió, atendido por, cajero, seller, sold by; o usuario/empleado/user solo si la tabla también tiene importe y venta), da un aviso (nunca un
  error) con la salida «ignora este aviso» si es lo que quieres. No cuenta vacíos, «N/A», guiones, «Sin …», filas de total ni al propio empleado de prueba.
- **Campo de formulario que la ruta exige y el formulario no trae.** En la ferretería la pantalla de usuarios enviaba `contrasena` y la ruta
  `/usuarios/nuevo` leía `request.form.get("password")` y rechazaba el envío si venía vacío: no se podía crear empleados desde la web y `verificar` no
  lo veía (solo miraba `request.form["k"]`, y daba el campo por presente con cualquier literal, como `type="password"` o `id="password"`). Ahora
  da un error propio, con los campos que sí trae el formulario, cuando la ruta lee `request.form.get("k")` y rechaza el envío si viene vacío
  (`if not v: return|raise|abort`, también con `.strip()`, `.lower()`, `.upper()` y `.title()`). Un campo solo cuenta como presente si está en el
  código (script, `{{ }}`, `on…=`) y no en `id=`, `for=`, `placeholder=` o `type=`; los `.js` locales que carga la plantilla cuentan, y si crean campos
  (`FormData`, `createElement`) callan el aviso.
- Medido sin el modelo, en 122-123 carpetas guardadas: la ferretería real da el error de `contrasena`/`password` y la corregida ninguno; 1
  acierto nuevo de campos (editar usuario enviaba solo `rol` y la ruta exigía `nombre`, comprobado con el cliente de Flask), 3 avisos nuevos de panel
  (vistos a mano: todos enseñaban ventas del administrador), 1 veredicto nuevo de contraseña, y 0 falsos en los tres; el análisis no tarda más
  (el análisis de campos: 4,2 s antes y 4,15 s ahora en 123 webs; la prueba con web: 12,8 s antes y 12,9 s ahora de media por web). Cambiando el `name=` de un campo en 39 webs con rutas
  validadas, 79 de 109 cambios se ven (con el criterio de antes, 32); de los 30 restantes, 17 son webs que `verificar` no lee con seguridad y 12 usan
  JavaScript o un nombre dinámico. **No medido:** nada de esto con el modelo. 762 pruebas del motor pasan (26 nuevas; 4 omitidas); ruff limpio en el
  motor y en los tests tocados. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.16] — 2026-11-21 (un `.bat` que llama a una herramienta de Aura o que promete resolver sin calcular ya no pasa)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10. Sale de una prueba real del horario de menor coste.

- **`.bat` que llama a una herramienta de Aura.** El `run.bat` de una misión lanzaba `resolver_sat problema.opb`, que es una herramienta de Aura (como
  `escribir` o `ejecutar`) y no un comando de Windows: con doble clic salía «"resolver_sat" no se reconoce», y ni `crear_bat` ni `verificar` lo
  veían. Ahora `crear_bat` no crea el archivo y devuelve un error que explica por qué fallaría y qué hacer, y `verificar` acusa la línea de un
  `.bat` que ya existe (también tras escribirlo o editarlo). No cuenta `git`, ni lo que el proyecto tenga como `.bat`, `.cmd` o `.exe` con ese
  nombre, ni las líneas de `echo`, `rem`, etiquetas o `python resolver_sat.py`.
- **`.bat` que promete resolver y solo muestra una solución guardada.** `resolver.bat` y el resumen de la misión decían «regenera el `.opb`, lo
  resuelve y muestra el resultado», pero solo se regeneraba el `.opb` y `decodificar.py` enseñaba el `horario.modelo` que Aura había calculado:
  con otros datos seguía mostrando la solución vieja (con el coste de un empleado cambiado, siguió dando $737). Ahora `verificar` da un error si el
  proyecto trae un `*.modelo` (la solución guardada por `resolver_sat`), un `.bat` que dice resolver, calcular u optimizar, y ningún código del
  proyecto, fuera de los tests, que calcule (con scipy, python-sat, z3, ortools, pulp, mip, cvxpy, highspy, pyomo, gurobipy o cplex, o que escriba
  el `.modelo`). El mensaje da las dos salidas: que el proyecto calcule con una librería y su `requirements.txt`, o que el `.bat` y el resumen digan
  «muestra la solución calculada».
- **Guía de matemáticas:** una frase nueva dice que lo entregado tiene que poder recalcular sin Aura y que `resolver_sat` nunca va en un `.bat`.
- Medido sin el modelo: la carpeta real del horario da 1 aviso y la corregida ninguno; 0 mensajes nuevos en 122 carpetas guardadas (195 `.bat`).
  **No medido:** la frase de la guía con el modelo. 736 pruebas del motor pasan (15 nuevas; 4 omitidas); ruff limpio en el motor y en los tests
  tocados. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.15] — 2026-11-21 (el motor SAT se actualiza solo al arrancar el agente)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10.

- **El motor SAT se actualiza solo.** La 4.13.14 fijó el motor SAT en la v0.2.3 (los `.opb` por planos de corte), pero el complemento solo se
  actualizaba al pulsar **Instalar motor**, así que una instalación con la versión anterior seguía con el motor viejo. Medido en un horario de 126
  variables (9 empleados, 7 días, 2 turnos): el motor anterior da coste 739 tras 600 s sin demostrar el óptimo; la v0.2.3 da 737, demostrado, en
  0,1 s. Ahora, al arrancar el agente, si el complemento está activado y su versión no es la fijada, baja la fijada (2 MB) en segundo plano, sin
  retrasar el arranque ni escribir en la salida del agente. No activa los que nunca se configuraron ni los que el usuario desactivó, respeta
  `AGENTE_SAT`, usa un candado para que dos proyectos abiertos no descarguen a la vez y, sin red, con una suma distinta, con un 404 o con el motor
  en uso, conserva el anterior y reintenta en el próximo arranque.
- Corrige la entrada de la 4.13.14 de este registro, que decía que el motor nuevo se descargaba al arrancar.
- Probado con diez condiciones fijadas antes de implementar (actualiza; no toca lo desactivado ni lo nunca configurado; no descarga si ya está al
  día; respeta `AGENTE_SAT`; cinco fallos sin ruido; candado; no retrasa el arranque; no ensucia el protocolo `--json`) y con el arranque real
  de `python -m agente --json` con el motor v0.2.2 activado y descarga real de la release: en menos de 25 s queda la v0.2.3 con su suma y
  resuelve un `.opb`. 721 pruebas del motor pasan (4 omitidas); ruff limpio en el motor y en el test tocado. Rust y la interfaz no cambian
  respecto a la 4.13.10.

---

## [4.13.14] — 2026-11-21 (verificar prueba la venta en la propia lista y con «Sin cliente»; el motor SAT pasa a la v0.2.3, que resuelve los .opb por planos de corte)

Cambia solo el motor; el programa de escritorio es el de la 4.13.10.

- **Venta hecha en la propia lista de ventas.** Seis de las webs guardadas venden desde `/ventas`, sin botón de «nueva venta», y `verificar` las
  descartaba. Ahora, si esa página trae algo que elegir (desplegable de producto, cantidades o un buscador) y un botón de vender (venta, vender,
  cobrar, finalizar, procesar, completar, confirmar, pagar, pedido, orden), la prueba vende en ella y comprueba que la venta quedó en la lista.
  Tres de esas webs pasan a venderse y una queda sin confirmar; a mano con un navegador real las cuatro venden. 0 mensajes nuevos.
- **Venta dejando «Sin cliente» como viene.** Quien vende sin tocar el desplegable de cliente es el caso más común y la prueba elegía siempre el
  primer cliente. Si la venta sale bien y el desplegable trae una opción por defecto de verdad (no «Selecciona…» ni vacía), la prueba repite la
  venta dejándolo como viene. Se acusa un 500, un aviso o un texto con error que no es un rechazo con razón (cliente, requerido, stock…), o enviar y
  no registrar sin decir nada. En una web guardada acusa un caso real: «FOREIGN KEY constraint failed» al vender sin cliente. La segunda venta
  corre en siete de las 58 webs con pantalla de venta y suma unos 0,9 s de media por web.
- **Motor SAT v0.2.3.** El complemento Sat-hibrido-rust pasa de la v0.2.1 a la v0.2.3. Un `.opb` se resuelve con planos de corte y VeriPB
  comprueba la prueba contra el propio `.opb`; el CNF no cambia. Con 67 instancias `.opb` (mochilas, coberturas, asignaciones, vértices, cajas,
  turnos e infactibles) y 30 s por instancia, demuestra 49 óptimos frente a 25 y VeriPB certifica 48 frente a 27; ningún resultado erróneo
  (tampoco en 600 `.opb` aleatorios). **Límite:** en mochilas de 100 a 200 objetos, si se agota el tiempo, la mejor solución puede ser peor que
  con la v0.2.1. Para recibir la v0.2.3 (2 MB) hay que pulsar **Instalar motor** (Configuración → Motor) después de actualizar; hasta entonces
  Aura sigue con el motor SAT anterior. Quien desactivó el complemento sigue sin él, y con una suma que no coincide o sin red se conserva el
  motor anterior.
- Medido sin el modelo, con las webs guardadas; cada mensaje o estado nuevo comprobado a mano con un navegador real. **No medido:** estos cambios
  con el modelo.
- 714 pruebas del motor pasan (4 omitidas); ruff limpio. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.13] — 2026-11-20 (verificar prueba más pantallas de venta: buscadores de productos, botones «False» y webs sin productos)

Sale de las ocho webs guardadas con pantalla de venta que `verificar` dejaba pasar en silencio con la 4.13.12 (no se sabía si en ellas se podía
vender). Cambia solo `verificar` del motor; el programa de escritorio es el de la 4.13.10.

- **Botón o enlace que dice «False», «True» o «None».** Si el texto visible de un `<button>`, de un `<input>` de enviar o de un enlace es solo
  uno de esos valores, sale un aviso: la plantilla pinta el valor de una variable de Python en vez de un texto (con WTForms,
  `{{ form.submit.data }}` es False hasta que se envía; hay que usar `{{ form.submit.label.text }}`).
- **El script de la pantalla de venta falla y no hay productos.** Si no hay desplegable de productos, no hay productos creados y el script de la
  pantalla da un error que no depende de los datos («X is not defined» o de sintaxis), sale como error. Si la página carga un
  `<script type="module">`, el mensaje (también el de «no aparece el desplegable» con productos) explica que las funciones de un módulo no son
  globales y un `onclick="función()"` no las ve.
- **Buscadores de productos con autocompletado.** La prueba de venta con navegador escribe «Aur», «Pro» o «Pru» en el buscador de productos,
  pulsa la sugerencia que sale (o deja que la fila se rellene sola) y sigue como siempre: cantidad, guardar y comprobar que la venta quedó en
  la lista. Antes callaba. De los cuatro buscadores guardados, tres venden y uno acusa un error real («Error al crear venta: list indices must be
  integers»).
- **Producto de prueba propio.** Si no hay productos al probar la venta, el recorrido da de alta uno desde el formulario HTML de la propia web
  (costo 7.77, venta 21.31, «Producto Aura»). Antes solo los creaba la prueba del empleado, que se salta si la web no tiene alta de usuarios
  con rol o si no se entra como administrador. La prueba de venta corre ahora en tres webs guardadas más.
- Medido sin el modelo: 122 webs guardadas, cada mensaje o estado nuevo comprobado a mano con un navegador real (los dos mensajes nuevos de los
  botones y del script, las tres ventas que ahora se prueban); 0 falsos, y el tiempo medio por web pasa de 12,3 s a 12,6 s. **No medido:**
  estos cambios con el modelo.
- 699 pruebas del motor pasan (4 omitidas: dos necesitan un Python con Flask); ruff limpio. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.12] — 2026-11-17 (verificar avisa de variables que ninguna ruta pasa y de pantallas de venta que no dejan vender)

Sale de dos pruebas: la veterinaria del usuario con la 4.13.8 (botones y listas que nunca aparecían sin que `verificar` dijera nada) y
cuatro misiones de negocio con la 4.13.11 (ninguna de las cuatro entregas dejaba vender y `verificar` daba 0 errores). Cambia solo
`verificar` del motor; el programa de escritorio es el de la 4.13.10.

- **Variables de plantilla que ninguna ruta pasa.** Jinja no da error por una variable sin definir: la lista sale vacía, el `if` es falso y
  el texto sale en blanco. Ahora `verificar` calcula con el Python del proyecto qué nombres lee cada plantilla y no define ni el entorno ni un
  procesador de contexto, y mira si alguna `render_template` se los pasa. Dice la línea, el efecto (bucle siempre vacío, parte que nunca se
  muestra, texto en blanco, error 500), qué sí pasa la ruta y un nombre parecido. Es error si hay un bucle (en una página sin JavaScript que cargue
  datos) o un atributo o llamada que da 500; aviso en el resto. En la veterinaria: `can_edit`, `vacunas`, `pacientes`, `cita`.
- **La pantalla de venta que no deja vender.** La prueba de venta con navegador callaba tres casos que dejaban el negocio sin poder vender:
  el formulario sin `method="post"` que el script envía con `this.submit()` (el navegador manda los datos por GET y la venta no se guarda),
  el desplegable de productos que no aparece porque el script falla al cargar (datos mal envueltos, `toFixed`), y la tabla de cantidades cuyos
  campos ocultos quedan fuera del `<form>` («agrega al menos un producto»). Ahora salen como error con el motivo. Además la prueba sabe poner una
  cantidad en las tablas de cantidades (tres webs que antes no se podían probar ahora venden en la prueba).
- Medido sin el modelo: variables sin pasar en 27 de 122 webs guardadas, 45 nombres revisados uno a uno, 0 falsos; pantalla de venta en 62 webs
  con prueba de venta más las 4 entregas de las misiones, con las 3 que no vendían y una del corpus (confirmada a mano), 0 falsos, +0,2 s por web.
- Medido con el modelo (4 misiones de negocio con Qwen3.6-35B-A3B y el motor de la variable sin pasar): el aviso saltó en una misión y el agente lo
  dejó a cero; en las otras no llegó a verlo (una verificó una sola vez en 150 pasos). Terminan 1 de 4 y ninguna entrega deja vender, por fallos de
  la pantalla de venta que son los que ahora detecta el segundo cambio. **No medido:** el segundo cambio con el modelo.
- 672 pruebas del motor pasan (4 omitidas: dos necesitan un Python con Flask); ruff limpio. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.11] — 2026-11-16 (verificar avisa de un campo que el formulario no trae y prueba al empleado con roles que no se llaman «empleado»)

Sale de la misma prueba real con la 4.13.8 (una veterinaria): crear usuarios y registrar vacunas daba error 400 desde la pantalla y
`verificar` no decía nada. Cambia solo `verificar` del motor; el programa de escritorio es el de la 4.13.10.

- **Error si una ruta exige un campo que su formulario no trae.** Cuando una ruta Flask lee `request.form["campo"]` siempre y un
  formulario POST que envía a ella no tiene ese campo, Flask responde 400 en cada envío y esa acción no se puede hacer desde la
  pantalla. En la veterinaria eran el alta y la edición de usuarios (`nombre_completo`) y el registro de vacunas (`nombre` y
  `fecha_aplicacion`, y el formulario enviaba `vacuna_id` y `fecha`). Ahora sale un error por formulario con la ruta y los campos que
  faltan. Es estático y calla si la web usa `flask_wtf`/`wtforms`, nombres de campo dinámicos, JavaScript que arma campos
  (`FormData`, `createElement('input')`…) o si no puede seguir un `include`.
- **La prueba del empleado también corre cuando ningún rol se llama «empleado».** Con roles como `recepcion` y `veterinario` se
  saltaba la prueba (no se comprobaba qué ve ese rol); ahora usa el primer rol que no sea de jefe.
- Medido sin el modelo, en 122 webs guardadas: el aviso nuevo solo salta en la veterinaria (4 campos en 3 formularios; enviando de
  verdad cada formulario con el cliente de pruebas de Flask da 400 con los campos del formulario y 302 con los de la ruta), 0 falsos,
  +0,04 s por web. Quitando uno a uno los campos exigidos de las webs que los traían, se detectan 55 de 57. La prueba del empleado con
  otros roles no cambia nada en esas 122 webs (casi todas llaman «empleado» a su rol).
- **No medido:** el efecto de estos avisos en las misiones con el modelo, ni su alcance fuera de la veterinaria (el resto de webs
  guardadas tienen formularios completos).
- 646 pruebas del motor pasan (2 omitidas); ruff limpio. Rust y la interfaz no cambian respecto a la 4.13.10.

---

## [4.13.10] — 2026-11-16 (arreglo: «Mostrar en la carpeta» no abría nada)

Arreglo de la 4.13.9, que salió con un botón que no funcionaba. No cambia el motor ni lo que se le dice al modelo.

- **«Mostrar en la carpeta» de la tarjeta «Resultado» abre el Explorador con el archivo seleccionado.** En la 4.13.9 no abría nada:
  el programa activaba `kill_on_drop` al lanzar el Explorador y lo soltaba al instante, así que `explorer.exe` moría antes de abrir
  ninguna ventana. Reproducido con un programa de prueba (0 ventanas con ese código; 1 ventana, con `run.bat` seleccionado, sin
  `kill_on_drop`).
- **Abrir una página local en el navegador del sistema (`open_external`) tenía el mismo fallo** y tampoco llegaba a abrir nada
  (en la prueba, 0 procesos nuevos frente a 2). Los dos se lanzan ahora con `std::process::Command` y se sueltan.
- Comprobado: la función real (`reveal_in_folder`) abre el Explorador con el archivo seleccionado (prueba manual `#[ignore]`
  `reveal_opens_explorer`, mirando la ventana con `Shell.Application`); las 21 pruebas de Rust de siempre, 22 de la interfaz y 621
  del motor pasan.
- **No medido:** el botón no se ha probado todavía con la app instalada (sí la función de Rust que llama).

---

## [4.13.9] — 2026-11-16 (la tarjeta de resultado dice dónde está el .bat y verificar avisa de enlaces a rutas que no existen)

Sale de una prueba real con la 4.13.8 (una veterinaria): el usuario no encontró el `.bat` que abre el sistema y el sistema tenía el botón
de registrar vacunas roto sin que `verificar` dijera nada.

- **La tarjeta «Resultado» enseña la ruta completa del lanzador** (la carpeta de trabajo más el `.bat`) en vez del nombre suelto:
  la carpeta de trabajo por defecto está en `AppData\Local`, oculta en el Explorador. Con dos botones: **Copiar ruta** y **Mostrar en
  la carpeta**. El segundo usa un comando nuevo del binario, `reveal_in_folder`, que solo acepta un archivo que existe dentro de la
  carpeta de trabajo (se canonicalizan las dos rutas) y abre el Explorador con el archivo seleccionado; cualquier otra ruta devuelve un
  error que la tarjeta enseña. Cambio de la interfaz y del binario de Tauri.
- **`verificar` da error si una plantilla usa `url_for('nombre')` y ninguna ruta se llama así.** En la veterinaria, los botones
  «Editar» y «Vacuna» llamaban a `url_for('editar_paciente')` y `url_for('nueva_vacuna')` cuando las rutas eran `paciente_editar` y
  `vacuna_nueva`, y estaban detrás de un `{% if can_edit %}` que nadie definía: la página daba 200 y no había forma de registrar una
  vacuna desde la web. La comprobación es estática: lee las rutas con `ast` (funciones decoradas, `endpoint=`, `add_url_rule`,
  `as_view`, Blueprints) y busca esos nombres en las plantillas; sugiere la ruta parecida. Calla si no está seguro de cuáles son las
  rutas (sin Flask ni FastAPI, rutas registradas de forma dinámica, extensiones que añaden rutas).
- Medido sin GPU, con condiciones fijadas antes:
  - `url_for`: 7 hallazgos en 3 de 122 webs guardadas, los 7 reales a mano (incluida la veterinaria); 0 falsos tras corregir dos
    durante el desarrollo (rutas registradas en un bucle y un nombre armado con `~`); +0,1 s de media por web.
  - Tarjeta: `tsc`, 22 pruebas de la interfaz y 21 de Rust (con una prueba nueva del comando); vista previa con un navegador (ruta
    corta, larga y fuera del proyecto) y el Explorador real selecciona el archivo con una ruta con espacios.
- **No medido:** el efecto de lo que `verificar` señala en las misiones con el modelo; y el comando `reveal_in_folder` no se ha
  llamado todavía a través de la app instalada (sí con pruebas unitarias y con el Explorador con los mismos argumentos).
  621 pruebas del motor pasan, 2 omitidas.

---

## [4.13.8] — 2026-11-16 (verificar comprueba que la venta se guarda, sigue los menús de JavaScript y prueba al empleado en más webs)

Salidas de las misiones de negocio con la 4.13.7: entregas con la venta que no se guardaba sin avisar, un desplegable de productos
vacío sin error visible, texto con marcadores sin sustituir y webs con el menú pintado por JavaScript que `verificar` casi no
recorría. Todo este paquete está en `verificar`, sin tocar la forma de trabajar del agente con Qwen3.6.

- **`verificar` comprueba que la venta hecha desde la pantalla se guardó:** compara la lista de ventas antes y después. Error si
  la petición salió y nada cambió con una pantalla idéntica, o si la pantalla dice «error / falló / no se pudo» sin motivo de
  rechazo legítimo. También abre el panel «Nueva venta» de la lista de ventas (un botón que no es un enlace). 3 de las 15 webs
  que la 4.13.7 daba por «guardada» no guardaban: «guardada» era solo la ausencia de queja.
- **Desplegable de productos vacío:** además de los errores de la página, avisa cuando la propia web lo cuenta con
  `console.error` (p. ej. `value="{{ productos | tojson }}"` entre comillas dobles, que el JavaScript no puede leer).
- **Marcadores de formato sin sustituir en las plantillas:** avisa de `${total:.2f}` o `{x:.2f}` escritos en el texto de una
  plantilla Jinja (Jinja no los sustituye y la persona ve el marcador). No mira el JavaScript, los estilos, `<template>`,
  `{% raw %}`, comentarios ni atributos.
- **El recorrido sigue las rutas de los menús que pinta JavaScript** (`{ href: '/productos' }` en un script del mismo origen).
  Antes, una web con el menú así quedaba con 4 de 17 páginas recorridas: no se daba de alta el producto de la prueba, no se abría
  «nueva venta» con productos y su 500 (`Object of type Row is not JSON serializable`) no salía. Una ruta que solo conoce el
  JavaScript y no existe (404, 405) no cuenta como enlace roto.
- **El empleado de la comprobación entra cuando el formulario de usuarios solo tiene `nombre`:** el alta lo creaba como
  «Empleado Aura» y luego se intentaba entrar como `empleado_aura`; la prueba de lo que ve el empleado no corría en 4 de las 28
  entregas recientes. Ahora corre en 41 de 121 webs (antes 37).
- Medido sin GPU, con condiciones fijadas antes y cada mensaje nuevo confirmado a mano:
  - Venta guardada y panel: 3 mensajes nuevos en 101 entregas, los 3 reales; +0,8 s de media por web.
  - Consola del desplegable: 1 hallazgo real, 0 cambios en el resto; +0,2 s por web. Marcadores sin sustituir: 4 de 75 webs con
    plantillas, los 4 reales.
  - Menús de JavaScript: 1 mensaje nuevo en 121 webs, real; +0,1 s por web. Solo 6 de unas 121 webs escriben rutas en JavaScript
    y una sola tiene el menú entero así.
  - Empleado con `nombre`: 0 mensajes nuevos; +0,2 s por web.
- **No medido con el modelo:** en 4 misiones de negocio hechas con el primero de estos cambios (m4150) no saltó ningún aviso
  nuevo y terminaron 0 de 4; el efecto en las misiones de lo que `verificar` señala sigue sin medirse. Se probó además lanzar
  `verificar` automáticamente durante la misión (12 misiones en tres pasadas) y no se incluye: no mejoró de forma demostrable.
  607 pruebas pasan, 2 omitidas.

---

## [4.13.7] — 2026-11-14 (verificar prueba ventas, informes y lo que ve el empleado)

Salidas de las 4 misiones de negocio con la 4.13.6: las entregas daban pantallas que nadie podía usar (una venta que no
se guardaba, una factura que reventaba con un 500, un informe en 0) y `verificar` las daba por buenas. Todo este paquete
está en `verificar`, sin tocar la forma de trabajar del agente con Qwen3.6.

- **`verificar` vende desde la pantalla de «nueva venta»:** con un navegador oculto entra, abre la pantalla de venta, elige un
  producto y una cantidad como lo haría una persona, guarda y mira. Error si el desplegable de productos queda vacío porque
  el script falla (con el error de JavaScript), si el servidor contesta «agrega al menos un producto» aunque se eligió uno
  (las líneas no llegan, p. ej. un campo fuera del `<form>`), o si guardar da un 500. Antes la venta solo se probaba con un
  envío directo del formulario, que saltaba el fallo de la pantalla. Corre antes del recorrido de siempre (ese recorrido
  cambia la contraseña del administrador al editar usuarios).
- **`verificar` avisa del informe en 0 y de las facturas vacías** cuando ya hay una venta: avisa si el informe por defecto
  enseña todos sus importes en 0 aunque hay ventas (una fecha de fin sin la hora, el mes, UTC frente a la hora local) y si la
  lista de facturas está vacía sin botón para generarlas (se crea y no se guarda). Son avisos, no errores.
- **El empleado de la comprobación, más lejos:** da un error si recibe un 500 donde el administrador abre bien la página, y
  avisa si, sin haber vendido nada, abre detalles o listas de ventas y facturas de otros (aviso: en algunos negocios los
  empleados ven todas las ventas). Solo si las páginas cerradas siguen cerradas.
- **Arreglo del falso «nadie puede entrar»:** un panel con el alta de empleados (usuario, contraseña, nombre) se tomaba por la
  pantalla de entrar y `verificar` decía que nadie podía entrar en una web donde `admin` entraba bien.
- **Argumentos inválidos con la firma:** si una llamada a una herramienta viene mal hecha, el mensaje nombra lo que falta y da
  el uso (`escribir(ruta, contenido, anadir=…)`). Pensado para otros modelos: Qwen3.6 no lo recibió nunca en 46 misiones.
- Medido sin GPU sobre 93 webs guardadas y 8 entregas nuevas, con condiciones fijadas antes y cada aviso confirmado a mano:
  - Venta en pantalla: caza las dos entregas rotas de la medición anterior (una con el desplegable vacío por una variable sin
    definir, otra con un 500 al guardar) y calla en las 15 webs donde la venta se guarda; corre en 24 de 93 webs. Al haber una
    venta real aparecen fallos que antes no se veían (el detalle de una venta que da 500 siempre, un informe en 0 por la fecha
    UTC). Añade 1,4 s de media por web (+126 s en las 93; el navegador son 3-6 s en las 24 con pantalla de venta).
  - Empleado: 8 avisos de ventas ajenas, los 8 reales; ningún 500 del empleado en las 101 webs; 0 falsos.
  - Informe en 0 y facturas vacías: 0 avisos nuevos en las 93 webs y 2 reales en las entregas nuevas; solo corre donde hay una
    venta con importe visible (11 % de las webs antes de la venta en pantalla, 12 % después).
  - Arreglo del falso «nadie puede entrar»: 0 cambios en las 93 webs; los 6 casos verdaderos siguen y desaparece el falso.
- **No medido con el modelo:** el efecto de estos avisos en las misiones (si Aura arregla lo que `verificar` le señala) no se
  ha medido todavía. 570 pruebas pasan, 2 omitidas.

## [4.13.6] — 2026-11-14 (guía de permisos por rol y comprobación del empleado en verificar)

Salidas de las mediciones de negocio con la 4.13.4 y la 4.13.5: en las 3 entregas que funcionaban, el empleado veía las
ganancias del administrador por dirección directa. Dos cambios, uno para que el modelo no lo haga y otro para que `verificar`
lo vea si lo hace.

- **Guía `roles` para negocios con empleados:** el agente la abre al leer «empleado», «permisos», «login»… Pide una tabla de
  roles en el README, rutas cerradas por defecto con el rol exigido también en el POST, una lista por pantalla (panel, listas,
  detalles, productos sin editar, API y JavaScript sin costo ni en `data-*`, reportes y usuarios), una pantalla de Usuarios con
  `generate_password_hash` y una prueba de marcadores (costo `7.77`, total `60.00`, nota `NOTA-ADMIN-77`) que entra como
  empleado y comprueba que no ve esos valores sin filtrar por el texto del marcador. Medida en 2 tandas de 4 misiones: no
  cierra todo (las ventas y facturas ajenas siguen abiertas), pero `/reportes` y el panel sin ganancias quedaron cerrados en las
  6 entregas que funcionaban (0 de 3 sin la guía).
- **`verificar` entra como un empleado:** si la web tiene pantalla de usuarios con rol, crea un producto con costo `7.77` y un
  usuario «empleado_aura» desde los formularios, entra como empleado y lee (solo GET) las páginas del administrador y las rutas
  de API habituales. Error si el empleado ve el costo, abre una sección de gestión con ganancias, abre el alta de usuarios o ve
  una sección igual que el administrador. Sin pantalla de usuarios con rol, con entrada por JavaScript o si el empleado no entra:
  silencio. Se prueba antes del recorrido de siempre y con sesión propia (el recorrido edita usuarios).
- Medido sin GPU sobre 93 webs guardadas: salta en 18, y las 18 son fugas reales (8 de costo y 10 de gestión); no hay falsos.
  Calla en las entregas ya cerradas. Añade 1,8 s de media. **Corre entera en 21 de 93 webs (23 %)**: el resto no tiene pantalla
  de usuarios con rol o da de alta y entra con JavaScript.
- Al medirla salieron 4 ajustes que ya van dentro: el empleado se prueba antes del recorrido, al entrar se sigue desde donde
  lleva la web, un informe de ganancias parcial también cuenta y los textos de prueba no son «Prueba» (chocaban con códigos
  únicos y daban un 500 falso). 531 pruebas pasan, 2 omitidas.
- Medido después en 4 misiones de negocio con el modelo (n=4, misma misión y carpetas vacías que las tandas anteriores): el
  agente pidió la guía en las 4; 3 de las 4 entregas no dejan al empleado ver costo, ganancias ni ventas ajenas con la lista
  estricta (antes, 1 de 4), sin que el modelo escondiera datos por el texto del marcador; la cuarta, la única que no escribió la
  prueba de marcadores, deja ver al empleado las facturas del administrador. Terminó 1 de 4 (mediana 39,2 min; las otras tres
  llegaron a 150 pasos) y cada una de las tres con permisos cerrados tiene otro fallo (informe de ganancias en 0 por defecto, error
  500 para el empleado al ver su venta, facturas que no se guardan). La comprobación del empleado corrió entera en 3 de 4, sin
  avisos y sin falsos; en una no pudo entrar (el único campo de nombre es el del login).
- No cubre ventas, facturas ni panel de otro usuario, que es justo la fuga real de la cuarta entrega.

## [4.13.5] — 2026-11-12 (verificar ve tests que borran la base, registros abiertos, tests sin correr y READMEs rotos)

Cuatro comprobaciones nuevas de `verificar`, salidas de la primera prueba real con la 4.13.4 instalada (un sistema de negocio que
funcionaba: entraba, vendía, bajaba el stock y daba la ganancia) y de las misiones de negocio. Todas se midieron sin GPU sobre 82
proyectos guardados, con condiciones fijadas antes y cada disparo revisado a mano.

- **Un test que borra la base real de la app:** error si un test hace `os.remove` o `unlink` de un archivo `.db`/`.sqlite` con el
  mismo nombre que usa el código de la app (y que no sale de `tempfile`): quien pulse `test.bat` pierde todo lo que guardó. En la
  entrega de la prueba, `test.py` borraba `instance/negocio.db` antes de cada prueba. Salta en 3 de los 82 proyectos, los 3
  reales.
- **Registro abierto a cualquiera:** aviso si una ruta `/register`, `/registro` o `/signup` sin ninguna comprobación de quién es crea
  cuentas en un sistema con roles. Salta en 2 proyectos, los 2 reales (en uno, el rol salía del formulario: un anónimo podía
  registrarse como administrador). No se avisa si crear la cuenta va dentro de un `if` que mira el usuario.
- **`test.py` y `tests.py` ya cuentan como pruebas** si definen una clase `TestCase` o funciones `test_*`: antes `verificar`
  decía «No hay tests» y no corría las 17 pruebas de la entrega de la prueba. Un `test.py` que solo lanza `test_app.py` no cuenta.
  Al correrlos salió un caso real: los tests de `m4134_neg3` fallaban (la venta no bajaba el stock).
- **Un README que cita un `.bat` o `.cmd` que no existe:** error («README.md cita install.bat, pero no existe»). Salta en 5
  proyectos, los 5 reales. No cuentan `activate.bat` de un entorno virtual, `npm.cmd`, las rutas con variables ni los que el README
  dice que se creen.
- Medido sin GPU: no hay falsos en los 82 proyectos tras tres ajustes hechos al ver los primeros resultados (una exclusión demasiado
  amplia en el borrado de bases, un registro que solo crea cuentas dentro de una comprobación de administrador y los `test.py`
  lanzadores de `test_app.py`, anotados en las condiciones). El aviso de
  `app.run(host="0.0.0.0", debug=True)` se midió y se descartó: saltaba en el 37 % de las entregas. **No se ha medido el efecto en
  misiones con el modelo.**
- No cubre que un empleado vea por dirección directa lo que no le toca (ventas ajenas, `/api/ventas`, precios de compra): depende de
  qué ve cada rol en cada negocio.

## [4.13.4] — 2026-11-12 (verificar ve más fallos de las webs que Aura entrega)

Ocho comprobaciones nuevas de `verificar`: cada una sale de un fallo real que una entrega de Qwen3.6 tenía y
`verificar` daba por buena. Todas se midieron sin GPU sobre 73 a 81 webs guardadas: solo cambian las webs que de verdad
tienen el fallo (el primer pase de «administrador sin permiso» y «altas por modal» dio cuatro falsos y una web mal
marcada; se corrigieron antes de integrar y quedó anotado).

- **El `.bat` de doble clic no sirve nada:** error si `python app.py` (lo que abre el `.bat`) crea la aplicación pero
  nunca llama a `app.run`, así que no se abre ningún servidor.
- **El `.bat` borra la base en cada arranque:** error si el `.bat`, o un `.py` que lanza, hace `drop_all`, `DROP TABLE` o
  borra el `.db` cada vez que arranca: quien lo abre pierde sus datos.
- **Venta con líneas hechas por JavaScript:** el recorrido pulsa «+ Agregar Producto» con un navegador oculto, rellena,
  guarda y da error si el servidor responde 500, deja una traza o la página enseña un error de programa con un 200.
- **Pantalla de entrar vacía:** error si `/` manda a una página de entrar sin formulario, campos ni botones.
- **Altas por modal:** si el botón «Nuevo» está fuera de todo formulario, el recorrido usa el navegador (hasta 40 s) y
  caza las páginas que dan 500 cuando ya hay datos.
- **Administrador sin permiso:** error si el administrador entra pero al abrir su sección la web responde 403 o lo manda
  a otra página con «No tienes permiso» (solo antes de enviar ningún formulario, para no confundirlo con los cambios de
  rol que hace el propio recorrido).
- **Pantalla de entrar en blanco:** error si la contraseña está en la página pero un contenedor con `display:none` la
  oculta y no se ve nada con lo que seguir; nombra el bloque culpable.
- **401 tras entrar:** error si, ya dentro, las lecturas (`GET` con `fetch`) de la pantalla reciben 401, o 403 siendo
  administrador: la petición sale sin credencial o el servidor rechaza la que le llega, y las listas salen vacías.
- Medido en misiones con Qwen3.6 (4 de negocio por tanda, 150 pasos): con las tres primeras comprobaciones, terminan
  3 de 4 (mediana 38,6 min) y 2 de 4 son usables; sumando las tres siguientes, terminan 3 de 4 (mediana 39,0 min) y 2 de 4
  son usables contando una con reserva (1 de 4 sin ella). **En esas ocho misiones ningún aviso nuevo llegó a saltar**, así
  que no se puede atribuir mejora ni empeoramiento a estos cambios. Las dos últimas (pantalla en blanco y 401) se midieron
  solo sobre webs guardadas, no en misiones. En esas misiones se vieron los dos fallos que motivaron las dos últimas.
- Solo cambia el motor de `verificar`: la interfaz y el resto del motor son los de la 4.13.3.

## [4.13.3] — 2026-11-11 (ingeniería inversa de APK y de JS minificado, en apps propias)

- **Aura sabe abrir, entender y parchear un APK que ya no tiene código** (apps propias o con permiso de su dueño):
  datos del paquete (permisos, servidor, versión), cambio de textos, colores o URLs y APK nuevo alineado y firmado.
  También lee el JS minificado de una web propia y corrige el fallo. Trae:
  - una guía `ingenieria-inversa` (alias apktool, jadx, smali…) que insiste en hacer solo lo pedido, trabajar sobre una
    copia, no descargar herramientas por su cuenta y no quitar licencias, DRM ni anuncios de apps ajenas;
  - `instalar_herramientas.bat`: baja apktool 3.0.3 y jadx 1.5.6 (88 MB) a `%LOCALAPPDATA%\AuraSentinel\herramientas`
    comprobando su huella SHA-256; el agente las tiene en el PATH y no necesita bajarlas de internet;
  - `verificar` con APK: que se reconstruye (errores de smali con archivo y línea), alineado, firmado y legible, y si hay un
    emulador encendido, que se instala, se abre y no se cierra.
- **Credenciales de `INSERT`:** `verificar` daba «nadie puede entrar» en falso cuando el usuario solo estaba en
  `INSERT INTO usuarios (nombre, username, password, rol) VALUES (…)`: tomaba el nombre como contraseña. Ahora cada valor
  va con su columna. Sin cambios de comportamiento en 65 webs guardadas.
- En las mediciones (`AGENTE_SIN_ESCRITORIO=1`) el agente ya no abre URLs ni páginas en el navegador del usuario.
- Medido con Qwen3.6 en 4 misiones de APK y web, comparadas con la 4.13.2: correctas 4/4 frente a 3/4 (la ganancia en
  acierto es pequeña). Lo que cambia de verdad: sin la herramienta, la 4.13.2 bajó apktool de internet sin comprobarlo y
  en una misión sobrescribió el APK original; con ella, cero descargas y originales intactos, y las misiones de cambiar
  un texto o un servidor van de 8,3 a 2,2 min y de 4,3 a 3,0 min. Repetida la consulta de datos (M1) con la guía
  corregida: 2 de 2 correctas, sin descargas, en 19 y 8 pasos. No se han vuelto a medir las misiones de negocio con la
  guía nueva.

## [4.13.2] — 2026-11-08 (entrar con las credenciales que da el proyecto, aunque no haya README)

- **Las credenciales que el proyecto da sin comillas o solo en el código ya sirven para entrar:** `verificar` daba
  «nadie puede entrar en la web» en falso en webs que sí entran con `admin` / `admin123`. Pasaba cuando la pareja no
  estaba en un README, sino en un comentario sin comillas o solo en el código (`hash_password('admin123')`):
  - ahora reconoce las parejas con etiqueta aunque vayan sin comillas («usuario: X … contraseña: Y», `Usuario=\`x\``,
    README con `**Usuario:**`);
  - prueba `admin` / `admin123` (lo que dan casi todas las webs de negocio) **antes** que las parejas sueltas que
    aparecen juntas en el código (`('Administrador', 'admin')` de los `INSERT`), que antes la empujaban más allá de las
    que se prueban, sin tapar las etiquetadas ni las parejas fiables que el propio proyecto da.
- **Prueba la app que abre el `.bat`, no el script de datos:** cuando `run.bat` lanza varios scripts (primero
  `seed.py` para crear la base de datos y después `app.py`), `verificar` probaba el primero y no la app. Ahora prueba
  el que arranca algo (Flask, `.run(`, `mainloop`…) o, si no, el último.
- Medido sin GPU sobre 57 webs guardadas, con condiciones fijadas antes: ningún cambio de comportamiento en las 57, y
  las webs de negocio que entran con `admin` / `admin123` sin README dejan de dar el error falso. Suite del motor en
  verde.

## [4.13.1] — 2026-11-08 (las webs que entran con JavaScript y los botones que no hacen nada)

- **Las webs que entran con JavaScript (`fetch`) ya se recorren por dentro:** en la 4.13.0, `verificar` solo avisaba
  de que no podía entrar en ellas. Ahora sigue con un navegador oculto (el mismo Playwright que ya usaba `verificar`):
  - entra con los usuarios y contraseñas que da el proyecto;
  - pulsa cada sección del menú, abre los formularios de «+ / Nuevo / Crear», los rellena con datos válidos y los
    guarda, y después da otra vuelta por las secciones;
  - da como error cada respuesta 500 del servidor, con su traza; los errores de JavaScript de la página son un aviso.
  Cancela los `confirm()`, no descarga nada y no pulsa salir, borrar ni cancelar. Las demás webs se recorren igual
  que antes.
- **Botones que al pulsarlos no hacen nada:** en varias webs de negocio, los botones «+ Nuevo producto» o «Editar»
  llamaban desde `onclick` a funciones de un script cargado con `type="module"`, que la página no ve; el navegador
  solo lo decía en la consola (`… is not defined`). Otra llamaba a funciones que no existían. Ahora `verificar` mira
  los `onclick`, `onsubmit`, `onchange`… del HTML, de las plantillas y del HTML que construyen los scripts, y da error
  si llaman a una función que no define ningún script del proyecto, o que está en un módulo sin `window.x = x`. El
  error dice dónde está la función y cómo arreglarlo. No ejecuta nada: 0,03 s por proyecto.
- Medido sin GPU sobre las webs guardadas, con condiciones fijadas antes de medir: el recorrido con navegador entra en
  4 de las 6 webs de JavaScript, sin errores falsos, y las otras 43 dan lo mismo que antes. La comprobación de los
  botones da 76 errores en 7 de 63 proyectos, todos reales (uno comprobado en el navegador), entre ellos la entrega de
  negocio que se usaba bien a mano en la 4.13.0, cuyos botones «Editar» no funcionaban.

## [4.13.0] — 2026-11-08 (verificar usa la web del proyecto de verdad)

- **Las webs se prueban usándolas, no solo arrancándolas:** en una misión de negocio, la web daba error 500 al
  registrar una venta. Sus tests pasaban, la revisión la aprobó y `verificar` solo comprobaba que el servidor
  arrancaba. Ahora, en las webs de Python (Flask, FastAPI, http.server), `verificar`:
  - arranca la web sobre una copia del proyecto, en un puerto libre y sin abrir el navegador;
  - entra con los usuarios y contraseñas que da el proyecto;
  - abre sus páginas y envía cada formulario con datos válidos (precios, stock, fechas, la primera opción de cada
    lista), y después vuelve a pasar por las páginas, ya con los datos creados;
  - da como error cada página o formulario que responde 500 o con una traza, o que va a una ruta que no existe, con
    la línea del proyecto donde falla;
  - con Flask, comprueba al arrancar que todas las plantillas del proyecto compilan, también las de páginas a las que
    el recorrido no llega.
  No pulsa salir, borrar ni cancelar.
- Las contraseñas que se pasan sin nombre a funciones como `crear_usuario("Administrador", "admin", "admin123")`
  también se usan para entrar.
- Medido: en 49 webs guardadas no da ningún error falso, tarda 1,3 s de mediana y encuentra fallos reales (500 o
  plantillas que no compilan) en 14 de ellas, entre ellas las 4 entregas de negocio de la versión anterior. En 4 misiones de negocio con Qwen3.6, el agente
  corrigió durante la misión 7 errores reales que le dio el recorrido; las 4 terminan, y 2 de las 4 entregas son
  usables a mano, frente a ninguna de las 4 de referencia.
- Las webs que entran con JavaScript (`fetch`) todavía no se recorren por dentro: `verificar` lo avisa.

## [4.12.9] — 2026-11-07 (el código que escribe el agente ya no puede borrar fuera del proyecto)

- **Un test generado vació la carpeta temporal del usuario:** en una misión de prueba, el modelo escribió un test que en
  `tearDown` borraba la carpeta que contiene su carpeta temporal (`shutil.rmtree(os.path.dirname(tempfile.mkdtemp()))`),
  es decir, todo `%TEMP%`, sin papelera, cada vez que se pasaban los tests. Ahora:
  - Los comandos del proyecto (tests, la app, pip...) usan su propia carpeta temporal, `.agente\tmp` dentro del
    proyecto, que no se puede borrar mientras el agente trabaja. Un test así solo puede vaciar esa carpeta.
  - Aura revisa el código Python antes de ejecutarlo. Si borra la carpeta temporal entera, la del usuario, una del
    sistema o el propio proyecto, avisa al escribir el archivo, `verificar` da error sin ejecutar los tests, y
    `ejecutar`, `experimentar` y `ver_pantalla` no ejecutan nada hasta que se corrija.
  - Si la tarea acaba con un borrado así sin corregir, el resumen empieza avisando de no abrir los tests ni el programa.
  - El servidor MCP también usa la carpeta temporal propia de cada proyecto.
  - La guía de tests pide `tmp_path`, `TemporaryDirectory` o `addCleanup`.
- Revisados 1051 archivos `.py` de 59 proyectos guardados: detecta el test de esa misión y no da ningún aviso falso.

## [4.12.8] — 2026-11-07 (verificar abre la app por el .bat, entra como cada usuario y compara lo que ve cada rol)

- **La app de escritorio se prueba por donde la abre el `.bat`:** en una misión de la 4.12.7, `run.bat` ejecutaba
  `python -m app`, pero la ventana se abría en `__main__.py` y no en `app.py`: con doble clic no se veía nada. `verificar`
  eligió `app.py` por el nombre y lo dio por bueno. Ahora prueba el archivo que abre el `.bat` de doble clic, y si ese
  archivo termina sin abrir ninguna ventana mientras otro sí la abre, es un error.
- **Cada rol tiene que ver lo suyo:** si el código habla de roles o permisos, la prueba de tkinter entra como hasta 3
  usuarios distintos y compara lo que ve cada uno (pestañas, botones y opciones de menú activos, y lo que dicen al
  usarlos). Si todos ven y pueden hacer exactamente lo mismo, es un error. También lo es dejar elegir el rol con botones
  «Entrar como…» sin pedir contraseña, como hacía otra misión de la 4.12.7. La prueba usa ahora también las opciones de
  los menús.
- **Si nadie puede entrar, es un error:** en una misión de prueba de esta versión la app creaba los usuarios solo cuando
  no existía la base de datos, pero la creaba antes de mirarlo, así que no había ningún usuario. Ahora, si el proyecto da
  usuarios y contraseñas y la pantalla de entrar los rechaza, `verificar` lo dice. La prueba escribe el usuario en el
  mismo formulario que la contraseña, no en campos ocultos de otra pantalla.
- En los 11 sistemas tkinter guardados caza esos fallos y los demás quedan igual. En 2 misiones de prueba no dio ningún
  error falso.

## [4.12.7] — 2026-11-06 (la ventana que desaparece al entrar es un error, y verificar no abre nada fuera de la app)

- **Una app que se queda sin ventana a la vista al entrar no pasa `verificar`:** en una misión de la 4.12.6, la prueba de
  tkinter cazó un fallo al entrar y el agente lo «arregló» construyendo la ventana principal en la misma ventana del
  login y ocultándola después: al entrar no se veía nada. La prueba solo dio un aviso, porque oculta ella misma las
  ventanas y no supo que había entrado. Ahora anota cuándo la app oculta o muestra sus ventanas: si al abrir o después
  de entrar no queda ninguna a la vista, es un error. También sabe que entró cuando la app oculta la ventana de entrar o
  la llena con la principal.
- **`verificar` ya no abre nada fuera de la app:** un botón «Exportar» que llamaba a `os.startfile` abrió Excel en el
  escritorio mientras la prueba usaba la app. Ahora abrir archivos, el navegador, el Explorador, el Bloc de notas o
  Excel se anota y no se hace.
- En los 6 sistemas tkinter guardados caza ese fallo y los demás quedan igual.

## [4.12.6] — 2026-11-06 (verificar usa las apps de escritorio con tkinter, y la contraseña del resumen sin rechazos falsos)

- **`verificar` usa las apps de escritorio hechas con tkinter:** en una misión de la 4.12.5, la peluquería terminó con
  `verificar` en verde y fallaba nada más entrar (las pestañas de la ventana principal se añadían de una forma que Tk no
  permite). Antes `verificar` no abría las apps con ventana, y si `main.py` solo importaba la app la daba por buena al
  llegar a su bucle de eventos. Ahora la usa sobre una copia del proyecto, sin enseñar ninguna ventana: los avisos y los
  diálogos se contestan solos; si hay pantalla de entrar prueba los usuarios y contraseñas que aparecen en el código;
  luego cambia a cada pestaña y pulsa cada botón que no sea de salir o borrar. Una excepción al abrir la ventana, al
  entrar o al cambiar de pestaña es un error, con la línea del proyecto; al pulsar un botón con los campos vacíos, un
  aviso. En los 4 sistemas tkinter guardados caza ese fallo y no da ningún error en los otros 3. PyQt, PySide y wx
  siguen sin abrirse.
- **La contraseña del resumen ya no se rechaza si el programa la pasa a una función de hash:** la comprobación de la
  4.12.3 no veía «empleada123» dentro de `hashlib.sha256("empleada123".encode())`, rechazó terminar dos veces y el
  resumen acabó diciendo «contraseña (ver en db.py línea 80)». En 18 resúmenes guardados con contraseña sigue marcando
  solo el caso real («admin» en vez de «admin123»).

## [4.12.5] — 2026-11-06 (ver_pantalla conserva la sesión, y otros modelos se pierden menos)

- **`ver_pantalla` conserva el inicio de sesión:** cada captura abría un navegador nuevo, así que una misión capturó
  `/login/admin` y luego `/agenda`, que salía como la pantalla de elegir rol, y gastó pasos escribiendo un script para
  probar con sesión. Ahora las cookies pasan de una captura a la siguiente dentro de la misma tarea; un encargo nuevo
  empieza sin sesión y «continúa» la conserva. Sirve para entrar con un enlace o una redirección; un formulario de
  usuario y contraseña todavía no se puede rellenar desde `ver_pantalla`.
- **Con ventana pequeña (menos de 32K) se compacta al 75 % en vez de al 60 %:** las instrucciones y herramientas ocupan
  unos 5.800 tokens fijos y con 16K solo quedaban unos 4.000 para trabajar; un modelo que no cabía en la VRAM
  compactaba cada pocos pasos. Con la ventana de Qwen3.6 (64K) no cambia nada.
- **Acabar con texto con el plan a medias pasa la misma puerta que `terminar`:** una misión con otro modelo dijo «He
  completado todas las fases» con un paso sin hacer, sin verificar ni revisar. En 125 tareas guardadas solo habría
  actuado en las 2 de esa misión.
- **Aviso si el modelo no está entrenado para herramientas** y contesta un encargo sin hacer nada.
- **Las métricas guardan con qué modelo y qué ventana se hizo cada tarea.**
- La prueba de `experimentar` ya no falla a veces en Windows.

## [4.12.4] — 2026-11-05 (el servidor viejo ya no tapa al nuevo, y los comandos de varias líneas se rechazan)

- **Un servidor viejo del agente en el mismo puerto se cierra:** el servidor de la parte anterior de una misión seguía
  en el puerto 5000 con el código de antes. Windows dejó escuchar también al nuevo y las peticiones iban al viejo, así
  que la misión reinició el nuevo 8 veces sin ver su cambio. Ahora, al lanzar un proceso en segundo plano (y al
  mirarlo con `procesos`), si un proceso más antiguo del agente escucha en el mismo puerto, se detiene y se avisa. Si el
  nuevo no llega a arrancar, se dice qué proceso del agente ocupa el puerto. Solo se tocan procesos del agente.
- **`ejecutar` rechaza los comandos de varias líneas:** cmd ejecuta solo la primera línea y calla el resto, así que un
  `python -c "…"` de varias líneas daba exit=0 sin salida y el agente creía que el código no imprimía. Ahora se le pide
  que use `experimentar` o un archivo `.py`. En las misiones guardadas había 111 comandos así, en 26 misiones.

## [4.12.3] — 2026-11-05 (tres fallos de una misión real: leer, .bat con Python y contraseña del resumen)

- **`leer` ya no deja al agente leyendo en bucle:** el modelo pedía `lineas=86` creyendo que era el número de línea
  (es cuántas líneas leer), recibía otra vez las líneas 1-86 y repetía hasta que el motor lo detenía sin contestar al
  usuario. Ahora, si ya leyó el principio y el archivo no ha cambiado, se le explica cómo leer desde la línea que busca.
  En 91 misiones guardadas pasó en 4, y en una se perdieron así 59 llamadas.
- **Un `.bat` que lanza `python` o `pip` sin buscarlo es un error de `verificar`:** `crear_bat` busca Python, pero una
  misión cambió su `run.bat` con `editar` por `python main.py` y con doble clic salía «"python" no se reconoce». Vale si
  el `.bat` recupera el PATH del usuario, usa `%PY%` o el `.venv`. En 164 `.bat` guardados marca 12, todos reales.
- **`terminar` se rechaza si el resumen da una contraseña que no está en el proyecto:** el programa creaba el usuario
  `admin` con `admin123` (así lo decían el código y el README) y el resumen final dijo «contraseña admin», así que la
  dueña no podía entrar. Si el proyecto no dice ninguna contraseña, no se comprueba. Entre 14 resúmenes guardados con
  contraseña, solo marca este.

## [4.12.2] — 2026-11-05 (verificar sin errores falsos en las webs con servidor)

- **Páginas sin su servidor:** `verificar` abre los `.html` como archivos sueltos, así que lo que solo da el servidor
  (una ruta `/api/...`, un `{{ url_for(...) }}` de una plantilla) fallaba y salía como error. Las misiones perseguían
  esos errores hasta quedarse sin pasos: de 12 misiones guardadas que no terminaron, 4 nunca verificaron sin errores por
  esto. Ahora las plantillas de servidor no se prueban como HTML suelto, y una petición que falla no es error si la ruta
  la define el servidor del proyecto (queda un aviso para probarla con el servidor en marcha). Una ruta que no define
  nadie sigue siendo error. Medido en 23 sistemas guardados: 31 errores, todos falsos, pasan a 0 y no aparece ninguno
  nuevo.
- **Atributos que lee una librería** (`conn.row_factory`, `app.secret_key`, `login_manager.login_view`…) ya no salen
  como «se define pero NUNCA se usa»: la revisión los convertía en CRITICO y rechazaba sistemas que funcionaban.
- **Un `.bat` que abre otro puerto que el del programa es un error:** un `run.bat` abría `http://localhost:5000` y la
  app escuchaba en 5055, así que con doble clic se veía «no se puede acceder». El puerto sale del código y la
  configuración del proyecto (y de los valores por defecto de Flask, uvicorn o `http.server`); sin puerto conocido, o con
  Docker, no se mira. En 40 sistemas guardados con `.bat` solo avisa en el caso real.

## [4.12.1] — 2026-11-04 (Los .bat del agente abren con doble clic)

- **Los `.bat` que crea el agente abren con doble clic:** con doble clic el `.bat` recibe el PATH del Explorador, que
  puede no traer Python, y decía «"python" no se reconoce» aunque el agente lo había probado bien. Ahora `crear_bat`
  busca los programas que lanza cada orden, también dentro de `start` y de `cmd /c` o `/k`; si falta alguno, recupera el
  PATH del usuario o usa el lanzador `py`, y si no lo encuentra lo dice y para. Medido: con 4.12.0, un `.bat` de
  `crear_bat` falla 4 de 4 sin Python en el PATH del sistema; con el arreglo abre 4 de 4.
- **`verificar` da error si un `.bat` lleva códigos de HTML** (`&gt;`, `&amp;`…), que cmd ejecuta tal cual.

## [4.12.0] — 2026-11-03 (Más margen para las misiones largas, y el motor ordenado por dentro)

- **150 pasos por defecto (antes 100):** con una orden corriente de sistema de negocio (productos, ventas, facturas,
  ganancias y roles), con 100 pasos terminaba 1 de 2 y no se podía usar; con 150 terminan 3 de 6 (medido en dos días; el
  primero salieron 3 de 3) y 2 de esos 3 se usan de principio a fin. Las tareas cortas acaban antes del límite y no
  cambian.
- **El motor, ordenado por capas:** sin ciclos entre módulos, cada conexión a la vista en la cabecera de su archivo y
  `motor/ARQUITECTURA.md` explica dónde va cada cosa; una prueba comprueba esas reglas. No cambia lo que hace el agente.

## [4.11.1] — 2026-11-01 (Circuitos, y fallos hallados en una misión matemática completa)

- **Motor SAT v0.2.1:** comparar dos circuitos iguales ya no se queda sin respuesta: dos multiplicadores de 16 bits
  pasan de sin respuesta en 30 s a menos de 0,1 s. No cambia ninguna orden. **Tras actualizar, pulsa Instalar motor.**
- **`resolver_sat` y `calcular` son herramientas, no módulos:** la guía y la descripción lo aclaran, y hay un aviso si el
  agente intenta importarlos.
- **Paradas y avisos falsos:** repetir `resolver_sat` tras cambiar el archivo ya no cuenta como bucle, y `conhost.exe`
  solo ya no se avisa como programa abierto.
- **Fórmulas rotas:** una cláusula vacía en DIMACS se rechaza en vez de dar INSATISFACIBLE, porque ese resultado no
  decía nada del problema.
- **`version`** ya admite respuestas grandes de PyPI (`python-sat` pasaba de 3 MB).
- **Guía de matemáticas:** circuitos, un multiplicador probado, constantes y formato `.opb`.
- **Aviso** a la 5.ª lectura del mismo archivo sin escribir nada.

## [4.11.0] — 2026-10-31 (Motor SAT v0.2.0 y optimización, y el agente respeta el código ajeno)

- **Motor SAT nuevo (Sat-hibrido-rust v0.2.0):**
  - `resolver_sat` usa `motor.exe` en vez de `hunt.exe`. Resuelve más problemas difíciles: en 95 fórmulas de prueba,
    88 frente a 86 y ninguna respuesta errónea; R(3,6) y R(4,4) en K18 pasan de sin respuesta a menos de 1 s.
  - **Optimización:** `resolver_sat` admite `.opb` (restricciones lineales con `min:`). Distingue el óptimo demostrado
    (ÓPTIMO) de la mejor solución hallada en ese tiempo (MEJOR ENCONTRADA), y comprueba el modelo y el coste contra el
    archivo. En problemas grandes con coeficientes muy variados puede no llegar al óptimo: la guía lo avisa.
  - Aura descarga la versión fijada `v0.2.0`, no la última publicada, porque las órdenes cambian entre versiones.
    **Tras actualizar, pulsa Instalar motor** (o Activar en Motores complementarios) para traerlo; hasta entonces el
    agente usa python-sat.
  - Ya no hay compilación de respaldo: si no se puede descargar, Aura lo explica y sigue sin el motor.
  - Un UNSAT y un óptimo no quedan certificados con VeriPB: lo normal es no tenerlo instalado (`--no-verify`).
- **El agente ya no borra código que nadie pidió tocar:**
  - `verificar` daba como error una función existente que nadie llamaba, y el agente la borraba para poder terminar.
    Ahora toma nota del código sin usar al empezar la tarea: eso sale como PREEXISTENTES y no bloquea. Lo que deje
    sin conectar la propia tarea sigue bloqueando, también si la retomas tras interrumpirla o reabrir Aura.
  - El revisor tampoco debe pedir borrarlo, salvo que la petición trate de eso.
- **Métricas por turno:** `.agente/metricas.jsonl` anota el tiempo y los tokens de cada turno y llamada, y
  `python -m agente.metricas <carpeta>` los resume. Solo se guardan en el proyecto; no salen del equipo.

## [4.10.1] — 2026-10-25 (Instalar motor en cualquier PC)

- **«Instalar motor» fallaba en la app instalada («"python" no se reconoce...»):**
  - Abierta desde el escritorio, la app recibe el entorno del Explorador, que en algunos equipos no lleva el PATH
    del usuario (pasa si es muy largo). `instalar.bat` no encontraba `uv` ni `python`.
  - `instalar.bat` busca `uv` también donde lo dejan sus instaladores (`%USERPROFILE%\.local\bin`, `.cargo\bin`,
    WinGet, scoop). Si no hay, descarga `uv` 0.13.0 de su repositorio oficial y solo lo usa si cuadra su SHA-256;
    `uv` crea el entorno con Python 3.12 y lo descarga si el equipo no lo tiene. Ya no hace falta instalar Python.
  - Versiones exactas de las dependencias en `motor/requisitos.txt` (las del entorno con el que pasan todas las
    pruebas): otro equipo queda igual que el de desarrollo.
  - La app y el motor completan el PATH con las carpetas guardadas del sistema y del usuario: el motor (también
    como MCP de Bionic), sus pruebas y los comandos del usuario encuentran `uv`, Python, `cargo`... aunque se
    abran desde el escritorio.

## [4.10.0] — 2026-10-25 (Cualquier modelo, control de procesos, apps Tauri abiertas y memoria)

- **El agente funciona con cualquier modelo, no solo con Qwen3.6:**
  - `agente.json` venía con `"modelo": "qwen3.6-35b-a3b"`. Desde Bionic (MCP) o la consola, el motor ignoraba el
    modelo cargado, lo descargaba de memoria y cargaba Qwen. Ahora viene en `"auto"`: usa el cargado o el elegido en
    Aura. Qwen3.6 sigue siendo el recomendado (el que se descarga y el que se propone si no hay ninguno cargado).
  - Lee la ficha de cada modelo en Bionic/LM Studio. Solo le pide no razonar si se puede apagar: a uno que razona
    siempre, como Flint, se le pedía igual. Avisa al empezar si el modelo no está entrenado para usar herramientas,
    si razona siempre o si está comprimido a 1-2 bits.
  - Contexto mínimo de 16K: sus instrucciones y herramientas ocupan ~5.000 tokens antes de empezar. Un modelo cargado
    a mano con menos (LM Studio pone 4096) se recarga con el mayor que quepa en la VRAM.
  - Entiende las llamadas a herramientas que los modelos escriben como texto: antes solo el formato de Qwen; ahora
    también Mistral (`[TOOL_CALLS]`), Llama (`<|python_tag|>`), Gemma (```` ```tool_code ````) y JSON suelto. Solo
    cuentan los nombres de herramientas que existen.
- **Probar modelo** (botón junto al selector de Aura, o `python -m agente --probar-modelo CLAVE`): carga el modelo si
  hace falta y le pide crear un archivo con una herramienta y terminar, como en una misión. Dice si es apto, apto con
  límites o no apto y por qué: si cabe en la GPU o irá a la RAM, contexto, velocidad, si llama a las herramientas
  bien o como texto, si razona siempre y si está muy comprimido. La memoria se mide con el modelo ya cargado (el que
  hubiera antes ocupa la RAM que usará) y la velocidad, en la segunda respuesta (la primera aún lee el modelo del disco).
- Al cargar un modelo que no cabe en la VRAM y no es MoE (Flint, 22,5 GB), el cargador decía «todo en VRAM»; ahora
  dice cuánto va en la VRAM y que el resto va en RAM y CPU.
- **El revisor conoce las prácticas actuales:** recibe, de las guías de los temas del proyecto (Tauri, Rust, pygame,
  web…), solo las frases que corrigen una versión vieja y nombran algo del código o de la petición (como mucho unos
  450 tokens). Si la petición nombra una API que no existe en la versión del proyecto (p. ej. `@tauri-apps/api` en
  Tauri 2), usar la forma actual cuenta como desviación justificada y no como CRÍTICO.
- **La gravedad se decide por la consecuencia:** CRÍTICO solo si, tal como está, algo falla o un requisito no se
  cumple. Un `[CRITICO]` que el propio revisor retira en su última frase («no rompe la funcionalidad», «esto es
  correcto») ya no bloquea el veredicto.
- **`verificar` revisa los ids del DOM también en TypeScript:** un `getElementById("x")` o `querySelector("#x")` en
  `.js`, `.ts`, `.tsx` o `.jsx` que busca un id que no tiene ningún HTML ni crea ningún script da error con
  `archivo:línea` y el id parecido que sí existe («¿Querías 'cpu-usage'?»). Antes solo se miraban los `.js` y los
  scripts dentro del HTML, y en Tauri la prueba no lo veía porque fuera de la app el código se detiene antes.
- Arreglo: `gen/` e `icons/` junto a `tauri.conf.json`, que Tauri regenera al compilar, ya no cuentan como código del
  proyecto. Sus esquemas (~300 KB) llenaban lo que lee el revisor y tapaban el código. En Aura-Monitor, lo que se le
  manda baja de unos 30 000 a unos 5 700 tokens.
- Arreglo: tras una revisión que pidió cambios, `terminar` sin haber tocado nada pasaba si la revisión era de la misma
  tarea (también un `revisar` a mano). Ahora se rechaza con los puntos pendientes. Y corregir solo un `.json` de
  configuración (`tauri.conf.json`, `capabilities/`) cuenta como cambio y lleva a la segunda revisión.
- **Las apps Tauri se prueban abiertas:** con todo lo demás sin errores, `verificar` abre la app de verdad unos segundos
  (el `.exe` de `cargo build`, con `dist/` servido en el puerto de `devUrl`) y dice qué se ve en su ventana, los errores
  de su consola y deja una captura. Sin cambios no la vuelve a abrir. `ver_pantalla` sin ruta (o con la URL de
  `devUrl`) enseña esa ventana. Antes miraba el frontend en un navegador, donde `invoke` no existe: salía sin datos y
  el agente «arreglaba» lo que funcionaba. Si no puede abrirla (sin compilar, `dist/` viejo, puerto ocupado) lo avisa y
  explica que el navegador no es la app.
- La guía de Tauri usa el puerto 1430 para el frontend en vez del 1420 habitual, que es el de Aura abierto en
  desarrollo: con los dos en el mismo puerto, la app no arrancaba.
- Arreglo: `crear_bat` pone `call` delante de `npm`, `npx` y los demás programas que en Windows son un `.cmd`. Sin él,
  `run.bat` hacía `npm install` y terminaba, sin llegar a `npx tauri dev`. `verificar` lo marca en los `.bat` escritos a
  mano.
- Arreglo: los comandos del proyecto ya no heredan lo que dejó para su propia compilación el programa que abrió Aura
  (`CARGO_TARGET_DIR`, `CARGO_PKG_*`, `npm_*`, `TAURI_CONFIG`...). Con Aura abierto en desarrollo, la app del usuario
  se compilaba en la carpeta de Aura. Las variables que el usuario tiene guardadas en Windows se respetan.
- Arreglo: `npm install` (o `add`, `uninstall`...) en una carpeta del proyecto sin `package.json` se bloquea: npm
  subía y cambiaba los paquetes de una carpeta superior, como la del usuario, que a menudo tiene uno. El agente crea
  antes el `package.json`.
- **El agente controla lo que abre y no cierra lo tuyo:** cada comando que lanza va en un «trabajo» de Windows (Job
  Object), que sabe exactamente qué procesos abrió, también lo que abre con `start`. Detener un proceso, que se pase de
  tiempo o cerrar Aura cierra solo eso. Antes se usaba `taskkill /T`, que decide por el número del proceso padre (Windows
  reutiliza esos números) y podía cerrar consolas y programas del usuario. Lo que un comando deja abierto (una ventana,
  el demonio de Gradle) sigue abierto como antes, pero sale en `procesos` como suyo y lo puede cerrar. Si el motor se
  cierra de golpe, Windows cierra con él lo que tenía en segundo plano; Aura ya no usa `taskkill /T` al detenerlo.
- `taskkill /IM x` o `/PID n` del agente cierran solo sus propios procesos con ese nombre o número. Los demás no se
  tocan: cerrar por número un proceso que no lanzó, o por nombre cuando no tiene ninguno suyo, pide permiso. Una misión
  cerró a la fuerza el proceso que ocupaba el puerto 1420: era la consola del servidor de Aura que el usuario tenía
  abierta.
- Arreglo: `restaurar` ya no llama «versión que funcionaba» a la que solo pasó `verificar`. Si después el usuario
  escribió algo (p. ej. «al iniciar con el .bat no arranca»), se lo recuerda al restaurarla: antes volvía a la versión de
  la que el usuario acababa de decir que fallaba, sin saberlo.
- **`investigar` recuerda lo que lee (memoria primero):** guarda los párrafos útiles de cada página con su fuente y la
  fecha en `motor\pasajes.db` (SQLite con búsqueda de texto completo, que encuentra nombres exactos como
  `invoke_handler`). Si una pregunta posterior está cubierta por lo ya leído, responde con ello al instante y sin salir a
  internet, diciendo de qué página es y cuándo se leyó. Antes volvía a buscar y leer las mismas páginas en cada tarea.
  Caduca pronto porque las APIs cambian: a los 30 días sin usarse o a los 90 de leerse; releer una página reemplaza lo
  guardado de ella. `investigar(pregunta, web=true)` lee internet aunque esté en memoria. Sin la base de datos,
  investiga en la web como antes.
- **Las lecciones que se usan sobreviven y cada tarea recibe solo las suyas:** antes iban siempre en el prompt de
  sistema los últimos 1.500 caracteres de `lecciones.md` (unas 8 lecciones, las más recientes, tuvieran que ver o no)
  y, con 40, se borraba la más antigua aunque fuera la que más servía. Ahora cada tarea recibe en su mensaje hasta 8
  lecciones que tienen que ver con ella: por un tema que nombra la tarea o que se deduce de los archivos del proyecto
  («arregla el botón» en un proyecto Tauri recibe las de Tauri), o por dos palabras con significado en común. Con la
  lista llena se olvida la que lleva más tiempo sin usarse; se usa al entrar en una tarea o cuando el agente la intenta
  guardar otra vez. El prompt de sistema ya no cambia al aprender, así que su prefijo sigue en la caché del servidor.
  `lecciones.md` sigue siendo la lista que se edita a mano; el uso va en `lecciones_uso.json`. Cuando una misión larga
  resume su historial, las lecciones de la tarea pasan al resumen (en el prompt de sistema nunca se perdían).

---

## [4.9.0] — 2026-10-18 (Apps de escritorio con Tauri 2 y arreglos de bucles)

- **Apps de escritorio con Tauri 2:** guía nueva (`guia("tauri")`) con la estructura completa escrita a mano (sin
  `tauri init`, que es interactivo), `tauri.conf.json` de Tauri 2 con su ventana, `build.rs`, permisos, iconos,
  `invoke` desde `@tauri-apps/api/core`, la CLI por npm y cómo comprobarlo sin abrir la ventana.
- **`verificar` revisa los proyectos Tauri:** avisa de lo que compila pero no funciona: sin ventana definida (la app
  arranca sin ventana), sin `build.rs`, sin `identifier`, sin `icons/icon.ico` o con uno que no es un ICO,
  `frontendDist` que no existe, un `#[tauri::command]` que no está en `generate_handler!`, un `invoke("x")` sin su
  comando, la API de Tauri 1, `window.__TAURI__` sin `withGlobalTauri` (dentro de la app vale `undefined`; se usa
  `isTauri()`), un comando que devuelve el JSON ya convertido en texto (`serde_json::to_string`: en JS llega una
  cadena), y el uso de CPU leído de un `System` recién creado (siempre 0 %).
- Arreglo: los archivos pedidos con varios puntos en el nombre (`tauri.conf.json`, `vite.config.ts`) se tomaban
  como `conf.json` o `config.ts`. El paso del plan no se podía marcar nunca y el agente acababa creando un
  `conf.json` de sobra para poder terminar.
- Arreglo: en proyectos Vite (también los de Tauri), `verificar` abría el `index.html` fuente, que carga `/src/main.ts`,
  y daba un error falso de «MIME type text/plain». Ahora prueba la página compilada de `dist/` después de
  `npm run build`, y si no se ha generado lo avisa.
- Arreglo: «Detenido: el modelo repitió 'verificar' 5 veces sin avanzar» saltaba aunque entre una verificación y
  otra hubiera ejecutado comandos nuevos (copiar un archivo, instalar). Ahora solo se detiene si repite lo mismo sin
  que haya cambiado nada.

---

## [4.8.0] — 2026-10-18 (Más rápido, revisar el plan y tarjeta de resultado)

- **Más rápido sin perder comprobaciones:** si el agente marca como hecho un paso de prueba sin haber probado tras su
  último cambio, el motor verifica él mismo en ese momento: si no hay errores, el paso queda hecho; si los hay, se los
  da directamente. Antes lo rechazaba y el modelo necesitaba dos vueltas más (verificar y volver a marcar).
- `git commit` en una carpeta sin repositorio lo crea y hace el commit en una sola llamada (nunca dentro de otro
  repositorio).
- `verificar` con una URL (`http://localhost:8000`) revisa el proyecto en vez de contestar «No existe».
- **Revisar el plan** (casilla junto al chat): el agente enseña el plan y espera a que lo apruebes o pidas cambios
  antes de tocar ningún archivo. Si pides cambios, rehace el plan y vuelve a preguntar.
- **Tarjeta de resultado** al terminar una misión: verificación, revisión, última captura, cómo abrirlo y los archivos
  que cambió (cada uno se abre con **Ver cambios**). Se construye con lo que ya mandó el agente, sin pedir nada al
  modelo.
- **Pedir al agente sobre esto** (menú contextual del editor o `Ctrl+Shift+A`): pone `@archivo (líneas a-b)` de la
  selección en el chat para que escribas qué quieres.

---

## [4.7.0] — 2026-10-18 (Modo grande: sistemas grandes y cambios en todos los módulos)

- **Modo grande** para mandatos que tocan todo un repositorio grande (p. ej. «añade inglés a toda la app»): el agente
  divide el trabajo en partes por módulos (15 archivos como máximo, cada una con su criterio de terminado y su prueba),
  hace cada parte en un contexto limpio, la verifica y la guarda en un commit local «Parte k/N». Entra con `/grande`,
  con «modo grande» o cuando el repositorio es grande y el mandato es transversal; tras cerrar Aura, «continúa» retoma
  la parte pendiente. Lee `ARQUITECTURA.md` y `CONVENCIONES.md` si existen. Nunca hace `push`.
- **`mapa`**: mapa del repositorio (módulos, archivos, líneas, clases y funciones) generado sin el modelo, con caché en
  `.agente\mapa.json` que solo vuelve a analizar lo que cambió. Solo se ofrece en repositorios grandes (≥ 25 archivos de
  código o ≥ 120 KB).
- **Idiomas en `verificar`**: claves que faltan en un idioma, textos vacíos, marcadores distintos, claves que usa el
  código y no existen, e idiomas copiados sin traducir (JSON, `.arb`, `.po`, YAML, `strings.xml`, `.resx`...). Solo
  corre si el proyecto tiene archivos de idioma.
- Los mandatos normales siguen igual: en repositorios pequeños las peticiones al modelo son idénticas a las de 4.6.2.
- Arreglo: `verificar(ruta=subcarpeta)` ya no marca como «NUNCA se usa» código que se usa en otra parte del proyecto.
- Juegos pygame: si la pantalla no cambia, `verificar` explica la causa (pantalla vacía, bucle sin
  `pygame.event.get()` o juego atascado, con las teclas enviadas y los textos en pantalla) y avisa si el juego cambia
  `SDL_VIDEODRIVER`.

---

## [4.6.2] — 2026-10-17 (archivos largos sin cortes ni HTTP 500)

- Arreglo: si el modelo escribía un archivo tan largo que la respuesta se cortaba a mitad de la llamada, el motor
  guardaba esa llamada con el JSON roto y el servidor del modelo contestaba «HTTP 500» a todo lo siguiente (también a
  «continúa»). Ahora la conversación nunca lleva JSON roto, el agente sabe que se cortó y escribe el archivo en partes
  con `escribir(..., anadir=true)`. El máximo por respuesta sube de 8.192 a 16.384 tokens.

---

## [4.6.1] — 2026-10-17 (Sat-hibrido-rust activado por defecto)

- Sat-hibrido-rust viene activado: `instalar.bat` (Configuración → Motor → Instalar motor) activa los motores
  complementarios predeterminados descargando su versión publicada con la SHA-256 comprobada. Si lo desactivas,
  reinstalar no lo reactiva; sin conexión, la instalación sigue y se activa después.
- Al compilar un motor desde su código ya no se descarga Chronos-VFS aparte: llega como dependencia de git de
  Sat-hibrido-rust.

---

## [4.6.0] — 2026-10-17 (Motores complementarios, blockchain, Android, bases de datos y aprender)

- **Motores complementarios:** los cerebros opcionales del autor se activan desde Configuración → Motor → Motores
  complementarios (o `python -m agente.complementos`), con enlace a su repositorio. El primero es Sat-hibrido-rust
  (`resolver_sat`), que usa Chronos-VFS. Al activarlo se descarga su versión publicada con la SHA-256 comprobada o se
  compila desde su repositorio, y se comprueba con dos fórmulas conocidas. `hunt.exe` ya no va dentro de Aura.
- **Blockchain:** guías `blockchain`, `evm` (Solidity con Foundry o Hardhat, varias redes EVM), `solana` (Anchor) y
  `mev` (arbitraje, liquidaciones y backrun con simulación en fork local). `verificar` compila y prueba con Foundry,
  Hardhat y Anchor (archivo:línea de cada error, tests que fallan, aviso de versiones de Anchor y Solana que no casan;
  `anchor test` solo contra Localnet).
- **Android y Flutter:** detecta el SDK, Android Studio, el Gradle descargado y los emuladores; los comandos reciben
  `ANDROID_HOME` y las herramientas del SDK. `verificar` ejecuta `assembleDebug` y las pruebas unitarias (o
  `flutter analyze`, `flutter test` y `flutter build apk`) y explica los fallos típicos. Nueva herramienta
  `abrir_android_studio` (si hay varias instalaciones, elige una completa); aceptar licencias pide permiso. `version` consulta el Maven de Google y pub.dev.
- **Bases de datos:** guía `bases-de-datos` (Docker, migraciones, varias bases); conectarse a una base de otra máquina
  pide permiso y borrar datos (`DROP`, `TRUNCATE`, `DELETE` sin `WHERE`, `down -v`...) pide confirmación.
- **Seguridad:** matar procesos por nombre (`taskkill /IM`, `Stop-Process -Name`, `pkill`, `killall`) pide
  confirmación: cierra los de todo el equipo (consolas, el IDE). Lo que lanza el agente se para con `procesos`.
- **Arreglos que destaparon las misiones reales:** un `.bat` que deja un proceso en segundo plano (`start /b`, un nodo local) ya no deja colgado al agente: la salida de los comandos va a un archivo temporal y no a una tubería. La verificación ignora las dependencias de Foundry (`lib/`, `cache/`, `broadcast/`) y no compila con javac el Java de los proyectos Android ni el que genera Flutter. Los `.bat` que crea el agente fijan `PYTHONUTF8=1`.
- **Aprender:** nueva herramienta `experimentar` (prueba mínima fuera del proyecto) y `aprender(leccion, tema)` guarda
  recetas comprobadas que `guia(tema)` muestra en las tareas siguientes.

---

## [4.5.0] — 2026-10-17 (Permiso para blockchain y matemáticas exactas)

- **Seguridad:** enviar transacciones o desplegar en una blockchain (`solana transfer`/`program deploy`, `spl-token`,
  `anchor deploy`/`upgrade`/`idl`, `forge script --broadcast`, `forge create`, `cast send`, `hardhat run --network`,
  `sui client`, `aptos move publish`) y `solana config set` piden permiso salvo contra un nodo local explícito. Sin red
  en el comando no se supone local: se lee la red por defecto de la CLI de Solana, el `cluster` de `Anchor.toml` y el
  `defaultNetwork` de Hardhat, y la ventana de permiso dice si es la RED PRINCIPAL o una red de prueba. Antes, estos
  comandos se ejecutaban sin preguntar.
- `hardhat verify` y `forge verify-contract` cuentan como publicar fuera del equipo. Los scripts de `package.json` que
  lanza `npm run` se revisan como los `.bat`.
- Nueva herramienta `calcular`: cálculo exacto con SymPy en un proceso aparte, con tiempo límite y solo cálculo.
  `instalar.bat` instala SymPy.
- Nueva herramienta `resolver_sat`: resuelve fórmulas DIMACS CNF con `motor\bin\hunt.exe` (motor Sat-hibrido-rust) y
  comprueba el modelo contra cada cláusula; errores de formato con su línea.
- Guía `matematicas` (SymPy y cómo codificar sudokus, horarios o coloreado en CNF) y reglas del agente para no
  calcular de memoria y trabajar con nodos locales en blockchain. El entorno indica Foundry, Anchor y la red por
  defecto de Solana.

---

## [4.4.0] — 2026-10-17 (Versiones de todos los ecosistemas y preguntas al usuario)

- `version` consulta también crates.io (Rust), el proxy de Go (con aviso si el módulo está obsoleto y explicación
  si es de la biblioteca estándar), NuGet (.NET, la mayor versión estable) y Docker Hub (etiquetas reales para `FROM`,
  sin `tip`, Windows ni `latest`, con filtro de variante como `golang:alpine`).
- Nueva herramienta `preguntar`: el agente pide un dato imprescindible o una decisión con 2-5 opciones y espera la
  respuesta (como mucho 3 veces por tarea; sin usuario, decide y lo explica). En Aura aparece la tarjeta «El agente
  pregunta» con botones y respuesta libre; en consola, opciones numeradas; en el protocolo, evento `pregunta`.
- La ventana de permiso muestra también el motivo (destructivo, o conectarse o publicar fuera del equipo).
- Guías de Rust, Go, .NET y Docker con la consulta de versiones de su ecosistema.
- Arreglo: la guía de Docker no llegaba a GitHub (una regla antigua del .gitignore ignoraba `docker.*` en
  todo el repositorio); ahora esas reglas solo valen en la raíz.

---

## [4.3.0] — 2026-10-17 (Sistemas en cualquier lenguaje, contenedores y despliegue)

- `verificar` comprueba también Rust (`cargo build`, `cargo test`, clippy), Go (`go vet`, `go test`, gofmt), .NET
  (`dotnet build`/`test`), Node/TypeScript (dependencias, `tsc --noEmit`, `npm run build`, `npm test`), Dockerfile
  (versiones fijas, `COPY` rotos, credenciales, usuario sin privilegios, `.dockerignore` y `docker build` con Docker en
  marcha), `compose.yaml` y tabuladores en YAML. Antes, un proyecto Rust, Go o .NET salía como «sin código» y un
  Dockerfile o un `npm test` rotos pasaban.
- La revisión, el mapa del proyecto, la protección contra reescrituras y los puntos de restauración incluyen `.rs`,
  `.go`, `.cs`, `.tsx`, `.yml`, Dockerfile, `go.mod`… (antes `restaurar` no podía devolver código Rust o Go).
- La revisión independiente recibe los comandos ejecutados en la tarea y su resultado: los requisitos que son
  acciones (desplegar, arrancar, comprobar) se juzgan con esa evidencia y no exige un script que nadie pidió.
- Permiso del usuario para todo lo que sale del equipo: `ssh`, `scp`, `rsync`, `git push`, `docker push`/`login`,
  `kubectl apply`, `helm`, `terraform apply`, publicar paquetes, nubes (`az`, `aws`, `gcloud`…) y peticiones HTTP que
  escriben fuera de `localhost`, también cuando van dentro de un `.bat`, `.ps1` o `.sh` que se ejecuta. Aura muestra
  el motivo en la ventana de permiso.
- Aviso al escribir un archivo con una credencial real (clave privada, token, contraseña); reglas para leerlas de
  variables de entorno o de un `.env` fuera de git.
- El agente conoce el entorno al empezar (Rust, Go, .NET, Docker y si está en marcha, Podman, OpenSSH, kubectl) y las
  compilaciones largas (`cargo`, `go`, `docker build`, `dotnet`, Maven/Gradle, `npx`) tienen al menos 5 minutos.
- Guías nuevas: `rust`, `go`, `node` (y TypeScript), `dotnet`, `docker` y `despliegue` (SSH sin interacción, servicios,
  contenedores, comprobación y vuelta atrás).
- Comprobado con misiones reales del modelo local en 8 GB de VRAM: una CLI en Rust, una API REST en Go con imagen
  Docker probada con curl y su despliegue por SSH en un servidor de prueba.

---

## [4.2.0] — 2026-10-16 (Motor en el repositorio e instalación completa)

- El motor (Agente Local) vive en `motor/`: clonar el repositorio trae todo lo necesario. No se suben su
  `.venv`, las medidas de VRAM de cada equipo ni las reglas y lecciones del usuario.
- `setup.ps1` instala un equipo nuevo de principio a fin: requisitos, interfaz, motor (entorno de Python,
  pruebas, registro MCP en Bionic/LM Studio), descarga verificada del modelo y ajuste a la VRAM en Bionic
  (contexto, capas de expertos en RAM/CPU, ajustes de chat). Se puede repetir.
- `motor\descargar_modelo.bat`: baja Qwen3.6-35B-A3B y su visión desde Hugging Face, reanuda los cortes y
  comprueba el SHA-256 publicado.
- Aura encuentra el motor del repositorio o el incluido en el instalador; si falta su entorno, Configuración →
  Motor ofrece **Instalar motor**. El instalador de Windows lleva el motor dentro.
- Icono propio (el cubo hexagonal de Aura) en la app, el instalador y la pestaña.
- Guía de instalación, integración con Bionic (qué cambia y cómo deshacerlo) y solución de problemas en
  `README.md` y `motor/LEEME.md`.
- Autoría y licencia MIT en `package.json`, `Cargo.toml` y el instalador (editor y copyright); guía de
  contribución, código de conducta, política de seguridad y plantillas de issues y pull requests en `.github/`.
- Instalador `setup.exe` publicado en GitHub (versiones) y guía para instalarlo sin compilar, con el aviso de
  SmartScreen y la comprobación SHA-256.
- Guía de instalación revisada desde un clon limpio: Git como requisito (`setup.ps1` avisa si falta), espacio
  real en disco (~30 GB) y qué hacer en equipos con menos memoria.
- La app ya no busca el motor en la carpeta de trabajo anterior a la unificación; eliminado `validate_project.rs`,
  que dependía del motor retirado.
- Arreglo: la interfaz se quedaba en blanco al usar la Terminal. En WebView2 reciente `scrollIntoView()`
  devuelve una Promise y un efecto de React la devolvía como si fuera su limpieza. Ningún efecto devuelve ya
  valores, y un límite de errores muestra «Recargar la interfaz» en vez de una ventana vacía (con prueba).
- Arreglo: las confirmaciones no esperaban respuesta. En Tauri `window.confirm` devuelve una Promise, así que
  «Nueva conversación», «Restaurar», «Deshacer cambios del agente», «Borrar perfil» y cerrar una pestaña sin
  guardar se ejecutaban sin preguntar. Ahora usan el diálogo nativo de Windows (`ask` con el permiso
  `dialog:allow-ask`), y Ctrl+W también pregunta si la pestaña tiene cambios (con prueba).

## [4.1.0] — 2026-10-16 (Interfaz nueva)

- Interfaz reescrita en React + TypeScript con Vite (antes un `main.js` de 1 200 líneas). Monaco 0.57,
  marked 18, DOMPurify 3.4.17; pruebas con Vitest.
- Panel del agente con estado, plan, tarjetas por paso, capturas, confirmación en línea y acciones rápidas;
  explorador con marcas de cambios del agente; aviso con diferencias y Deshacer en el editor; terminal,
  problemas, procesos y vista previa; paleta de comandos; búsqueda; git; puntos de restauración; reglas.
- Backend: evento `agent-event`, órdenes rápidas `agent_order`, `ide.rs` (estado del sistema con VRAM, git,
  capturas, búsqueda, terminal, copia del proyecto por misión).
- Instalador de Windows (NSIS y MSI) con identificador propio `com.yemsy.aurasentinel`.
- Rendimiento: Monaco en diferido (paquete inicial de 4,3 MB a 358 KB; primer pintado en ~0,1 s), sin
  consultas de sistema con la ventana oculta, binario con LTO y sin símbolos. Medido en la versión final:
  236 MiB de RAM con WebView2, 0 MiB de VRAM; misión real desde la interfaz en 1,2 min.

## [4.0.0] — 2026-10-16 (Agente Local como único motor)

- Las misiones las ejecuta Agente Local con el modelo de Bionic/LM Studio (`src-tauri/src/agent.rs`).
- Retirado el motor original basado en Ollama (~49 000 líneas: `llm/`, `core/`, `memory/`, `net/`,
  SpectraSAT, el submódulo Chronos-VFS, `Dockerfile`, `setup.sh`, `patch.py`, `data/`). Con él desaparecen
  sus 6 pruebas que fallaban; quedan 14 pruebas Rust y 3 de interfaz, todas en verde.
- Fuera el botón "Máxima potencia" y el selector de motor; el selector de modelos lista los de Bionic.
- Reanudar ahora detecta planes de Agente Local con pasos pendientes; el árbol de archivos ya no se vacía
  si una carpeta (p. ej. de OneDrive) no se puede leer.
- `start.bat` y `setup.ps1` comprueban Bionic y Agente Local en vez de Ollama.

---

## [3.0.0] — 2026-09-06 (Arquitectura v3)

### Selector de modelo unificado y modo turbo
- **Selector unificado**: un único selector de modelo en la barra superior, sincronizado con el backend de Tauri; se
  eliminaron los controles redundantes.
- **Modo turbo** («Máxima potencia»): omite micrometas redundantes y acelera los bucles de ejecución directa.
- **Indicador de estado**: temperatura, descarga en GPU y estado de compilación en tiempo real.

### Control de fases dinámico (PESP v2) y detección de entregables
- Los proyectos web ya no exigen `index.html`: se detectan los archivos HTML (`has_html_files`) y los entregables con
  otros nombres (`cyber_sentinel.html`, `dashboard.html`, etc.).
- El control de fases comprueba que los entregables existen y tienen contenido antes de pasar de fase.
- Si un script de verificación (p. ej. `verify_*.py`) termina con código 0 y todo en verde, la misión termina
  (`TOOL_FINISH`) en lugar de seguir consumiendo pasos.

### Subsistema AutoValidator reconectado
- `core::auto_validator::AutoValidator` vuelve a formar parte de la validación del espacio de trabajo
  (`validate_workspace`).
- Comprueba las etiquetas `<script src="...">`, `<link href="...">` y los recursos locales antes de la verificación
  final.
- Evita insertar código incompatible al generar recursos de aplicaciones web puras.

### Saneador de contexto y limpieza
- Eliminadas las rutas y marcadores obsoletos escritos a mano (`proxy-stack-windows`).
- El contexto de cada sesión filtra las rutas de otros proyectos del disco.
- La compilación de Rust queda con 0 errores y 0 avisos (`cargo check` en menos de 4 s).

---

## [0.5.0] — 2026-08-30 (Sprint 4)

### Nuevo — Arquitectura de Autonomía Completa (Tier 0)

#### SpectraSAT FFI — Motor Lógico Matemático en RAM
- Integración del motor SpectraSAT como librería nativa Rust vía FFI.
- El agente resuelve problemas SAT/UNSAT sin delegar al LLM, eliminando alucinaciones matemáticas.
- Retorna JSON estructurado con veredicto (SAT_CERTIFIED / UNSAT_*) y asignación booleana exacta de variables.
- Pipeline de 4 capas: GF2 Algebraico → SDP Relajado → Branch-and-Bound + Verificador Booleano → DPLL Exhausto.
- Corregido bug crítico de falso-positivo SAT: el motor ahora verifica cada asignación contra todas las cláusulas antes de certificar.

#### Zero-Hint Router
- El Planificador ahora deduce semánticamente qué herramienta usar sin que el usuario la mencione.
- Vocabulario matemático añadido al clasificador de intenciones: auditoría, satisfacibilidad, SAT, lógica.
- Misiones matemáticas ahora clasificadas como ANÁLISIS, evitando el protocolo de fases de construcción.

#### Mission Persistence (core/mission_persist.rs)
- Checkpoint de FSM (contexto, rol, paso) guardado a disco cada 3 pasos.
- auto_resume_if_needed() en setup() detecta misiones interrumpidas al reiniciar la aplicación.
- Evento mission-resumed emitido al frontend para notificar al usuario.

#### TOOL_CONTAINER (core/container.rs)
- Soporte nativo para Docker y Podman mediante auto-detección de runtime.
- Acciones: run, exec, stop, logs, activate_env.
- activate_env detecta tipo de proyecto (Node, Python, Rust) y activa el entorno virtual correcto.
- Handler TOOL_CONTAINER integrado en la FSM del Ejecutor.

#### Memoria Episódica Multi-Sesión (core/episodic_memory.rs)
- Cada misión completada genera un episodio persistente en ~/.aura_episodes.jsonl.
- Al iniciar una nueva tarea, se inyectan los últimos 3 episodios relevantes al contexto del agente.
- Búsqueda semántica simple por coincidencia de texto en el objetivo.
- Comandos Tauri: search_episodes_cmd, get_recent_episodes_cmd.

#### Scheduler Autónomo (core/scheduler.rs)
- Motor de tareas programadas usando cron expressions estándar (5 campos).
- Loop Tokio con tick cada 60 segundos, ejecutado en el runtime de Tauri (`tauri::async_runtime::spawn`).
- Herramienta TOOL_SCHEDULER disponible para el Planificador.
- Comandos Tauri: schedule_task, list_scheduled_tasks, remove_scheduled_task.

#### Sanity Monitor (core/sanity_monitor.rs)
- Evaluación de salud del agente cada 5 pasos de ejecución.
- Detecta: bucles de herramientas, presión de RAM (>90%), fallos JSON consecutivos, estancamiento.
- Emite nivel de alerta (GREEN / YELLOW / RED) con recomendaciones al contexto.
- Evento sanity-report emitido al frontend para el panel de telemetría.

### Corregido
- Bug: `tokio::spawn` en el scheduler causaba pánico al inicio (`no reactor running`). Cambiado a `tauri::async_runtime::spawn`.
- Bug: Extractor de cláusulas usaba `rfind("]]")` frágil. Reemplazado por contador de profundidad de corchetes.
- Bug: Planificador clasificaba auditorías matemáticas como misiones de Construcción, activando protocolo PESP de 4 fases y causando pausas interactivas innecesarias.
- Bug: Variables retry_tracker y agent_workspace eliminadas accidentalmente de scope en refactor anterior.

### Modificado
- session_journal.rs: Campos fsm_context, fsm_role, fsm_step, interrupted añadidos.
- core/mod.rs: Registrados los 5 módulos nuevos del Sprint 4.
- main.rs: start_scheduler() y auto_resume_if_needed() integrados en setup().
- agent.rs: Prompts del Planificador y Ejecutor actualizados con catálogo completo de herramientas.
- .gitignore: Excluidos binarios de prueba, Modelfiles, archivos de sesión runtime.

---

## [0.4.0] — 2026-07-15 (Sprint 3)

### Nuevo
- Vision Evaluator (xcap + moondream) para análisis visual de interfaces.
- Command Trail System para rastrear comandos ejecutados.
- Script Runner Generator automático por lenguaje de proyecto.
- Prompts comprimidos a <200 tokens optimizados para modelos 7B.
- Frontend Anti-Backend Shield.

---

## [0.3.0] — 2026-07-01 (Sprint 2)

### Nuevo
- Transición FSM Ejecutor → Crítico por micrometas.
- TOOL_AUDITOR con salida JSON estructurada.
- Anti-Stub Enforcer para bloquear código incompleto.

---

## [0.2.0] — 2026-06-15 (Sprint 1)

### Nuevo
- FSM Multi-Agente (Planificador, Ejecutor, Crítico).
- Clasificador de Tipo de Misión (NLU heurístico).
- Contrato de Aceptación entre roles.
- Compresión de contexto cada 10 pasos.
