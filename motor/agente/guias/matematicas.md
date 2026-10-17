# Matemáticas (cálculo exacto y problemas de restricciones)
- Nunca calcules de memoria ni "a ojo": usa `calcular(codigo)` (SymPy) y copia su resultado.
- `calcular`: Python con todo SymPy importado y `x, y, z, t` definidos; se muestra la última línea. Otros símbolos: `a, b = symbols('a b', positive=True)`.
  - Ecuaciones: `solve(x**2 - 5*x + 6, x)`, sistemas `solve([x + y - 3, x - y - 1], [x, y])`, reales `solveset(sin(x), x, Interval(0, 2*pi))`, lineales `linsolve(...)`, diferenciales `dsolve(Eq(f(t).diff(t), f(t)), f(t))` con `f = Function('f')`.
  - Cálculo: `diff(expr, x)`, `integrate(expr, (x, 0, 1))`, `limit(expr, x, oo)`, `series(expr, x, 0, 6)`, `summation(1/k**2, (k, 1, oo))` con `k = symbols('k', integer=True)`.
  - Álgebra: `simplify`, `expand`, `factor`, `apart`, `Matrix([[1, 2], [3, 4]]).det()`, `.inv()`, `.eigenvals()`, `.rref()`.
  - Números: `Rational(1, 3)` (exacto), `N(expr, 50)` (50 cifras), `isprime`, `factorint`, `gcd`, `mod_inverse`, `binomial`, `factorial`.
  - Optimización de una función: `solve(diff(f, x), x)` y comprueba con la segunda derivada y los extremos del intervalo.
- Restricciones y combinatoria (sudoku, horarios, coloreado de grafos, asignar personas a turnos, planificación): SAT.
  1. Una variable booleana por decisión, numeradas desde 1. Ej.: celda (f, c) con valor v en un sudoku 9x9: `81*f + 9*c + v` con f, c en 0..8 y v en 1..9.
  2. Cláusulas (listas de literales; `-n` = no n): "al menos uno" = `[a, b, c]`; "como mucho uno" = `[-a, -b]` para cada par; "exactamente uno" = las dos; `a → b` = `[-a, b]`; un dato fijo = `[a]`.
  3. Un script `codificar.py` escribe `problema.cnf` en DIMACS: primera línea `p cnf VARIABLES CLAUSULAS` y una cláusula por línea acabada en ` 0`.
     ```python
     import itertools
     clausulas = []
     def exactamente_uno(lits):
         clausulas.append(list(lits))
         clausulas.extend([-a, -b] for a, b in itertools.combinations(lits, 2))
     # ... restricciones ...
     n = max(abs(l) for c in clausulas for l in c)
     with open("problema.cnf", "w") as f:
         f.write(f"p cnf {n} {len(clausulas)}\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in clausulas))
     ```
  4. `resolver_sat("problema.cnf")`: SATISFACIBLE deja el modelo en `problema.modelo` (línea `v` con los literales; positivo = verdadero). INSATISFACIBLE = no hay solución con esas restricciones.
  5. Un script `decodificar.py` lee la línea `v`, reconstruye la solución y la COMPRUEBA contra el enunciado (filas, columnas, turnos...), no solo contra la CNF.
  6. Antes del caso grande, prueba la codificación con uno pequeño de solución conocida. Si sale INSATISFACIBLE y no debería, sobra una restricción o hay un signo cambiado.
- Si el programa del usuario debe resolver SAT él solo (sin el motor): `version("python-sat", "pypi")` (paquete `pysat`) o `z3-solver` para restricciones con números; en otros lenguajes, investiga su librería.
- Tests: comprueban la solución decodificada contra el enunciado y un caso sin solución.
