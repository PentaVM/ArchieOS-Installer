export const dynamic="force-dynamic";
export default function Login(){return <main className="login"><p className="eyebrow">ARCHIEOS / ENGINEERING</p><h1>Acceso privado</h1><p>Estado técnico del proyecto.</p>
  <form method="post" action="/api/login"><label htmlFor="password">Contraseña</label><input id="password" name="password" type="password" autoComplete="current-password" required maxLength={256}/><button type="submit">Entrar</button></form>
  <p className="muted">La sesión caduca a las cuatro horas.</p></main>;}
