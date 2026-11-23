export function DataTable({ id, caption, head, rows }: { id: string; caption: string; head: string[]; rows: (string | number)[][] }) {
  return (
    <table id={id}>
      <caption>{caption}</caption>
      <thead>
        <tr>{head.map((h) => <th scope="col">{h}</th>)}</tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr>{r.map((c, i) => (i === 0 ? <th scope="row">{c}</th> : <td>{c}</td>))}</tr>
        ))}
      </tbody>
    </table>
  );
}
