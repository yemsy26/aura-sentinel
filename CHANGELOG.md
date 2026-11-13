# Registro de cambios — Aura Sentinel

Todos los cambios relevantes del proyecto se documentan aquí. Formato: `[versión] — fecha`, con
[versionado semántico](https://semver.org/lang/es/).

---

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
