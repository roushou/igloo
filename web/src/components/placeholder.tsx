/** A page whose content a later task fills in. */
export function Placeholder({ title, detail }: { title: string; detail?: string }) {
  return (
    <section>
      <h1 className="text-xl font-semibold">{title}</h1>
      {detail ? <p className="mt-1 font-mono text-sm text-muted-foreground">{detail}</p> : null}
      <p className="mt-4 text-sm text-muted-foreground">Nothing here yet.</p>
    </section>
  );
}
