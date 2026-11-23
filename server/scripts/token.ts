import { createProjectToken, revokeToken } from "../src/auth.ts";
import { connect, migrate } from "../src/db.ts";

const [cmd, a, b] = process.argv.slice(2);
const usage = () => {
  console.error(
    "usage: token.ts create <public-name> [label] | revoke <token-id> | claim <project-id> <user-handle>",
  );
  process.exit(2);
};
if (
  !(
    (cmd === "create" && a) ||
    (cmd === "revoke" && /^\d+$/.test(a ?? "")) ||
    (cmd === "claim" && a && b)
  )
)
  usage();

const sql = connect(process.env.DATABASE_URL ?? "");
await migrate(sql);
let code = 0;
if (cmd === "create") {
  const r = await createProjectToken(sql, a as string, b);
  console.log(`project_id: ${r.projectId}\ntoken_id: ${r.tokenId}\ntoken: ${r.token}`);
} else if (cmd === "claim") {
  const r =
    await sql`update projects set owner_id = u.id from users u where projects.id = ${a as string} and lower(u.handle) = lower(${b as string}) returning projects.id`;
  if (r.length) console.log("claimed");
  else {
    console.error("no such project or user handle");
    code = 1;
  }
} else if (await revokeToken(sql, Number(a))) {
  console.log("revoked");
} else {
  console.error("no such active token");
  code = 1;
}
await sql.close();
process.exit(code);
