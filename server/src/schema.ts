import { z } from "zod";

const Hash = z.string().regex(/^[0-9a-f]{64}$/);
const u32 = z.number().int().min(0).max(4_294_967_295);

export const LedgerEntrySchema = z
  .object({
    v: z.literal(1),
    ruleset: z.string().min(1).max(32),
    action_id: z.string().min(1).max(256),
    ts: z.number().int().nonnegative(),
    project: z.string().min(1).max(200).nullable(),
    kind: z.string().min(1).max(32),
    verified_s: u32,
    unverified_s: u32,
    reasons: z.array(z.string().max(64)).max(32),
    features: z
      .record(z.string().max(64), z.number().int())
      .refine((o) => Object.keys(o).length <= 32),
    prev_hash: Hash,
    hash: Hash,
  })
  .strict();
export type LedgerEntry = z.infer<typeof LedgerEntrySchema>;

export const UploadBody = z
  .object({ prev_hash: Hash, entries: z.array(LedgerEntrySchema).min(1) })
  .strict();

export const CommitBody = z.object({ hash: Hash, ts: z.number().int().nonnegative() }).strict();
