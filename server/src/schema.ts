import { z } from "zod";

const Hash = z.string().regex(/^[0-9a-f]{64}$/);
const u32 = z.number().int().min(0).max(4_294_967_295);
// NUL breaks Postgres text; a lone surrogate breaks serde_json on the JSON round trip to the wasm verifier.
const str = () =>
  z.string().refine((s) => s.isWellFormed() && !s.includes("\0"), "no NUL or lone surrogates");

export const LedgerEntrySchema = z
  .object({
    v: z.literal(1),
    ruleset: str().min(1).max(32),
    action_id: str().min(1).max(256),
    ts: z.number().int().nonnegative(),
    project: str().min(1).max(200).nullable(),
    kind: str().min(1).max(32),
    verified_s: u32,
    unverified_s: u32,
    reasons: z.array(str().max(64)).max(32),
    features: z.record(str().max(64), z.number().int()).refine((o) => Object.keys(o).length <= 32),
    prev_hash: Hash,
    hash: Hash,
  })
  .strict();
export type LedgerEntry = z.infer<typeof LedgerEntrySchema>;

export const UploadBody = z
  .object({ prev_hash: Hash, entries: z.array(LedgerEntrySchema).min(1) })
  .strict();

// Cc: control characters; Cf: format characters (bidi overrides, zero-width), as for display names.
export const DeviceCodeBody = z
  .object({
    device_name: z
      .string()
      .min(1)
      .max(64)
      .refine((s) => !/[\p{Cc}\p{Cf}]/u.test(s), "no control or format characters"),
  })
  .strict();
export const DeviceTokenBody = z.object({ device_code: z.string().min(1).max(128) }).strict();

export const CommitBody = z.object({ hash: Hash, ts: z.number().int().nonnegative() }).strict();
