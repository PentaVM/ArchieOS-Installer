# Revisión — PR #35 (IRIS Beta 2.1 en Rust) y parte de #38 (Codex)

## PR #35 `claude/laughing-lamport-5af9cs` → main

13 commits, ~9180/-165, 56 archivos. CI: `native-contracts` ✅ y `rust` ✅; `mergeable=true`.

**Frontera de autoridad — preservada.** `archie-iris-runtime/src/lib.rs` documenta que el
runtime pasa por "policy y el Stage gate (`archie-iris-bridge`), exactamente como el servidor
de Python". `act.rs` es configuración de inferencia, no actuación directa: no vi `Command::new`,
`spawn` ni rutas que salten leases/Policy dentro del crate.

**Egreso/privacidad — alineado.** `93f2b34` enruta voz y arena por el router del registro y el
`Veil`: `PrivacyGrade` (Public/Internal/Sensitive), grado por defecto **Public** (conservador,
solo sale lo público/propio), `ask_private` retiene el texto cuando las reglas lo bloquean
("withheld by privacy rules"). Mismo patrón que la destilación de la LNCM.

**Memoria/Wanda — afirmación honesta a nivel unitario.** El test (`7792b74`) comprueba que un
dicho hablado persiste tras un reinicio **y** sigue marcado "no verificado": un hecho guardado
no se promueve a lazo verificado por el hecho de almacenarse (hearsay sigue siendo hearsay).
Cubre el lado Rust (`live/persistent.rs`) y el C++ (`sentry/store` + `identity/wanda`).

**Carga de pesos — segura, con nota menor.** `weights.rs` verifica SHA-256 del archivo antes de
leer (`expected_sha256`) y parsea un conjunto acotado de variantes pickle (`Int/Long/Float/Bool/
Unicode`) hacia un `ModelConfig` tipado, no un unpickler general que ejecute `REDUCE`/`GLOBAL`
para construir objetos arbitrarios. Como el `.pt` es el release propio del proyecto y está
fijado por digest, es aceptable. Nota menor: mantener el lector restringido a ese formato de
almacenamiento y no aceptar `.pt` de terceros sin digest.

**No verificado (correcto según AGENTS.md):** fps/latencia en la RTX y la build CUDA en hardware
real. `7ded078` afirma "compila y enlaza" (build/link), no fps medidos — consistente. No declaré
nada de hardware como verificado.

**Veredicto:** sin hallazgos bloqueantes. Listo para integrar en cuanto el dueño lo apruebe; la
única acción de seguimiento es la nota del lector de pesos (no bloqueante).

## Parte de Codex en #38

Las 51 commits incluyen el trabajo de `wanda_bonsai2` de Codex. La prueba FP32 estricta y la
admisión de entrenamiento (`f558637`, integrada como `9d56807` con el conflicto de `worker.py`
resuelto) quedaron contenidas: `admission.py`/`verification.py`/tests idénticos, importaciones de
`worker.py` como superconjunto, `precision_profile` hilado por `extract_cached`. CI verde.
Pruebas Python pasan salvo las que necesitan `torch` (GPU, no instalado en el contenedor).
