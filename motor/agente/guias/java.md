# Java (JDK 17+, 2026)
- En este equipo hay JDK 17 (`java`, `javac`). Maven/Gradle no están instalados: proyectos pequeños se compilan con javac. Si hacen falta librerías o JUnit, pide instalar Maven (`scoop install maven` o `choco install maven`).
- Estructura sin Maven: `src\app\Main.java` con `package app;`. Compilar: `javac -encoding UTF-8 -d out src\app\*.java`. Ejecutar: `java -cp out app.Main`. Con Maven: `src/main/java/...`, `src/test/java/...`, `mvn -q test`, `mvn -q package`.
- Java moderno: `record Punto(int x, int y) {}`, `var`, switch con flechas (`case A -> ...`), text blocks `"""`, `List.of(...)`, streams con moderación, `Optional` para valores ausentes.
- Entrada: `try (var sc = new Scanner(System.in)) { ... }` y valida con `hasNextInt()`. Para probar sin teclado: `echo 5| java -cp out app.Main`.
- Errores: captura excepciones concretas, no `catch (Exception e) {}` vacío. Mensajes claros al usuario.
- GUI: Swing con `SwingUtilities.invokeLater(() -> new Ventana().setVisible(true));`. Juegos: un `JPanel` con `paintComponent` y un `javax.swing.Timer` (16 ms) para el bucle; teclado con `KeyListener` o key bindings. Separa la lógica del dibujo para poder probarla.
- run.bat: `crear_bat("run", ["javac -encoding UTF-8 -d out src\\app\\*.java", "java -cp out app.Main"])`.
- `verificar` compila con -Xlint (errores y avisos con línea) y ejecuta el main sin pantalla (`-Djava.awt.headless=true`).
