import type { ApiError } from "../../lib/api";

const FRIENDLY_SLUGS: Record<string, string> = {
  invalid_yaml: "The YAML could not be parsed",
  empty_match: "Rule must include at least one filter",
  foreign_person_id: "A referenced person does not belong to your account",
  unwritable_album: "The target album does not exist or is not writable",
  id_conflict: "A rule with this id already exists",
  id_mismatch: "The id in the YAML does not match the URL",
  invalid_status: "Status must be one of active, paused, archived",
  empty_patch: "Nothing to update",
  not_found: "Rule not found",
  no_immich_key:
    "Connect your Immich account before creating rules (Setup → Immich)",
  resolver_error:
    "Could not reach Immich to validate the rule — please try again",
  network_error: "Network error — is the server running?",
};

export function humanRuleError(err: ApiError): string {
  const slug = err.error ?? "unknown_error";
  const friendly = FRIENDLY_SLUGS[slug];
  const detail = typeof err.detail === "string" ? err.detail : null;
  if (friendly && detail) return `${friendly}: ${detail}`;
  if (friendly) return friendly;
  if (detail) return `${slug}: ${detail}`;
  return slug;
}
