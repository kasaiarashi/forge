import { useParams } from 'react-router-dom';

/**
 * Combined `<owner>/<repo>` param helper for the namespaced routes.
 *
 * Pages now route under `/:owner/:repo/...`. Most page code wants the full
 * "alice/forge" string as a single value (to pass to the API), so this hook
 * glues the two URL segments together. Falls back to the bare `:repo`
 * segment if `owner` is absent (defensive — should not happen with the
 * current router config, but keeps single-segment legacy URLs working).
 */
export function useRepoParam(): string {
  const params = useParams<{ owner?: string; repo?: string }>();
  const owner = params.owner ?? '';
  const repo = params.repo ?? '';
  if (owner && repo) return `${owner}/${repo}`;
  return repo;
}
