"""El contrato de las herramientas: qué puede pedir el modelo y con qué argumentos. Solo datos; la ejecución está en
herramientas.py. Un cambio aquí cambia lo que el modelo ve: afecta a todas las misiones."""


def _f(nombre, desc, props, req):
    return {"type": "function", "function": {"name": nombre, "description": desc,
                                             "parameters": {"type": "object", "properties": props, "required": req}}}


S, I, B = {"type": "string"}, {"type": "integer"}, {"type": "boolean"}
LISTA = {"type": "array", "items": {"type": "string"}}

ESQUEMAS = [
    _f("plan", "Crea o reemplaza el plan de trabajo (3-10 pasos concretos: requisitos, tests reales, verificar, revisar, .bat).",
       {"pasos": LISTA}, ["pasos"]),
    _f("paso_hecho", "Marca un paso del plan como hecho.", {"numero": I, "nota": S}, ["numero"]),
    _f("listar", "Lista archivos y carpetas del proyecto.", {"ruta": S, "profundidad": I}, []),
    _f("leer", "Lee un archivo con números de línea (por bloques).", {"ruta": S, "desde": I, "lineas": I}, ["ruta"]),
    _f("buscar", "Busca texto o regex en los archivos del proyecto.", {"patron": S, "ruta": S, "extension": S}, ["patron"]),
    _f("escribir", "Crea o reemplaza un archivo con su contenido COMPLETO (sin '...' ni TODO). Valida sintaxis .py/.json. "
                   "Archivo largo (más de ~250 líneas): escríbelo en partes; la primera normal y las siguientes con "
                   "anadir=true, que las añade al final.",
       {"ruta": S, "contenido": S, "anadir": B}, ["ruta", "contenido"]),
    _f("editar", "Reemplaza un fragmento exacto de un archivo existente (copia el texto tal cual de leer).",
       {"ruta": S, "buscar": S, "reemplazar": S}, ["ruta", "buscar", "reemplazar"]),
    _f("ejecutar", "Ejecuta un comando de Windows (cmd) en la carpeta del proyecto. Usa fondo=true para servidores "
                   "o programas que no terminan. Si existe .venv se usa automáticamente.",
       {"comando": S, "segundos": I, "fondo": B}, ["comando"]),
    _f("procesos", "Ver o detener procesos lanzados con fondo=true.",
       {"accion": {"type": "string", "enum": ["listar", "ver", "detener"]}, "id": I}, ["accion"]),
    _f("crear_bat", "Crea un .bat robusto (UTF-8, se detiene en errores, crea .venv e instala requirements.txt si usar_venv).",
       {"nombre": S, "comandos": LISTA, "descripcion": S, "usar_venv": B}, ["nombre", "comandos"]),
    _f("web_buscar", "Busca en internet (documentación, errores, librerías). Devuelve títulos, URLs y resúmenes.",
       {"consulta": S}, ["consulta"]),
    _f("web_leer", "Lee el texto de una página web. 'buscar' filtra párrafos con esas palabras.",
       {"url": S, "buscar": S}, ["url"]),
    _f("investigar", "Busca en internet y lee las mejores páginas en UN paso: devuelve los párrafos relevantes con su "
                     "fuente. Úsalo para APIs, errores o librerías que no conozcas bien. Primero mira lo ya "
                     "investigado (se guarda 30 días); web=true lee internet aunque esté en memoria.",
       {"pregunta": S, "web": B}, ["pregunta"]),
    _f("calcular", "Cálculo matemático EXACTO con SymPy (nunca calcules de memoria): álgebra, ecuaciones (solve), "
                   "derivadas, integrales, límites, series, matrices, factorización, primos, fracciones y decimales "
                   "con la precisión que pidas (N(expr, 50)). 'codigo' es Python con todo SymPy importado y x, y, z, t "
                   "ya definidos; se muestra el valor de la última línea. Ej.: solve(x**2 - 5*x + 6, x)",
       {"codigo": S, "segundos": I}, ["codigo"]),
    _f("resolver_sat", "Llámala tú directamente: no se importa desde un script ni es un comando. Resuelve un .cnf (DIMACS) o un .opb "
                       "(restricciones lineales, con min: para optimizar) del "
                       "proyecto con el resolvedor del motor. Para restricciones y combinatoria (sudoku, horarios, "
                       "coloreado, asignaciones, planificación): codifícalas con un script (guia('matematicas')). "
                       "Devuelve SATISFACIBLE u ÓPTIMO con el modelo comprobado (en el .modelo) o INSATISFACIBLE.",
       {"archivo": S, "segundos": I, "modelo": S}, ["archivo"]),
    _f("guia", "Buenas prácticas ACTUALES y cómo verificar para: python, pygame, html-juego, web, api, java, rust, go, "
               "node, dotnet, docker, despliegue, android, flutter, tauri, blockchain, evm, solana, mev, bases-de-datos, "
               "matematicas, tests, bat, ingenieria-inversa (abrir y parchear un APK ya hecho, JS minificado). Incluye lo "
               "aprendido en tareas anteriores. Llámala al empezar un proyecto.", {"tema": S}, ["tema"]),
    _f("version", "Última versión real de un paquete o imagen (evita librerías obsoletas): pypi (y si hay wheel para tu "
                  "Python), npm, maven, crates (Rust), go (ruta del módulo, p. ej. github.com/gin-gonic/gin), nuget "
                  "(.NET), docker (imagen de Docker Hub, p. ej. python o node:alpine), google (Maven de Google para Android, "
                  "grupo:artefacto, p. ej. com.android.tools.build:gradle) o pub (Dart/Flutter).",
       {"paquete": S, "ecosistema": {"type": "string", "enum": ["pypi", "npm", "maven", "crates", "go", "nuget", "docker",
                                                                 "google", "pub"]}},
       ["paquete"]),
    _f("verificar", "Control de calidad de TODO el proyecto: errores, código que nunca se usa (funciones sin conectar), "
                    "tests y ejecución real (Python y pygame sin ventana, web en navegador, Java, Rust, Go, .NET, "
                    "Node/TypeScript, Dockerfile y Compose). Úsalo tras construir y tras cada corrección.",
       {"ruta": S, "segundos": I}, []),
    _f("ver_pantalla", "Captura la interfaz (un .html, una URL http://localhost:PUERTO o el .py de un juego pygame) y "
                       "comprueba si se ve bien: texto cortado o sobre fondo cargado, elementos fuera de pantalla, "
                       "pantalla en negro. En una app Tauri, sin ruta abre la app de verdad y enseña su ventana. "
                       "Si el modelo cargado ve imágenes, la revisa él. 'pregunta' = qué mirar además.",
       {"ruta": S, "pregunta": S, "segundos": I}, []),
    _f("revisar", "Revisión independiente: otro análisis compara cada requisito con el código y señala lo que falta o "
                  "no funciona. 'enfoque' = qué mirar con más cuidado.", {"requisitos": S, "enfoque": S}, []),
    _f("restaurar", "Vuelve a la última versión que pasó verificar sin errores (checkpoint). Úsalo si tus cambios "
                    "rompieron algo que funcionaba y no consigues arreglarlo.", {"checkpoint": I}, []),
    _f("aprender", "Guarda lo aprendido para futuras tareas: una lección de una frase tras resolver un error no obvio, "
                   "o con 'tema' (android, evm, docker...) una receta COMPROBADA que se añadirá a guia(tema).",
       {"leccion": S, "tema": S}, ["leccion"]),
    _f("experimentar", "Prueba mínima fuera del proyecto (.agente/experimento) para comprobar una API, una librería o "
                       "una idea antes de usarla: python, node, powershell o bat. Si funciona, aplícalo y guárdalo con "
                       "aprender(leccion, tema).", {"codigo": S, "lenguaje": S, "segundos": I}, ["codigo"]),
    _f("abrir_android_studio", "Abre un proyecto Android o Flutter en Android Studio para que sincronice Gradle e "
                               "instale los componentes del SDK que falten. Úsalo al crear el proyecto o si verificar "
                               "dice que falta el SDK, una plataforma o el wrapper.", {"ruta": S}, []),
    _f("git", "Git del proyecto (local, nunca push): estado, diff (cambios sin commit), log, commit (añade todo y "
              "confirma; 'mensaje' obligatorio), rama (crea o cambia a 'nombre'), otro ('comando' tras 'git'; lo "
              "destructivo pide confirmación). 'ruta' limita a un archivo o carpeta.",
       {"accion": {"type": "string", "enum": ["estado", "diff", "log", "commit", "rama", "otro"]}, "mensaje": S,
        "nombre": S, "ruta": S, "comando": S}, ["accion"]),
    _f("preguntar", "Pregunta al usuario y espera su respuesta. Solo si falta un dato imprescindible (servidor, "
                    "credencial, una decisión que cambia el resultado) o hay que elegir entre caminos: da 2-5 'opciones' "
                    "cortas. No preguntes lo que puedes decidir tú con una opción razonable.",
       {"pregunta": S, "opciones": LISTA}, ["pregunta"]),
    _f("terminar", "Termina la tarea cuando todo está hecho, verificado y revisado. Resume qué se hizo y cómo usarlo.",
       {"resumen": S}, ["resumen"]),
]
# Fuera de ESQUEMAS: mapa solo se ofrece en repos grandes (en los pequeños la petición al modelo no cambia) y plan_partes
# solo en la fase de plan del modo grande.
ESQUEMA_MAPA = _f("mapa", "Mapa del repositorio desde una caché al día: módulos, archivos, líneas y clases/funciones de "
                          "primer nivel. Úsalo ANTES de listar o leer. Sin 'ruta': resumen por módulo; con 'ruta' "
                          "(carpeta o archivo): el detalle.", {"ruta": S}, [])
PARTE = {"type": "object", "properties": {"titulo": S, "archivos": LISTA, "terminado": S, "prueba": S},
         "required": ["titulo", "archivos", "terminado", "prueba"]}
ESQUEMA_PLAN_PARTES = _f(
    "plan_partes", "Guarda el plan del mandato grande: partes en orden (lo común primero), cada una con 15 archivos como "
                   "máximo (rutas reales; los nuevos también), su criterio de terminado comprobable y su prueba. "
                   "'convenciones' = decisiones comunes que seguirán todas las partes.",
    {"partes": {"type": "array", "items": PARTE}, "convenciones": S}, ["partes"])
