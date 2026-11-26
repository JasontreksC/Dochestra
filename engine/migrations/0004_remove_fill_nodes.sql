-- D-7 (2026-11-26): the `fill` (form auto-fill) node was removed. Existing workflows and run records must not
-- keep it: an unknown node kind makes the graph (and with it GET /api/workflows/{id}, GET /api/runs/{id}) fail
-- to load.
--
--   1. workflows.graph:          drop kind='fill' nodes and every edge that touches one of them
--   2. runs.graph_snapshot:      the same
--   3. node_runs:                delete the rows of kind='fill'
--
-- Only rows that contain a fill node are rewritten (the WHERE clauses), so every other workflow / run keeps its
-- bytes (and its updated_at: workflows.updated_at is deliberately not touched). Nodes and edges keep their order.
-- Nodes that were fed by a fill node stay; their input is simply unconnected, as after deleting the node in the editor.

CREATE OR REPLACE FUNCTION pg_temp.strip_fill_nodes(graph jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE AS $$
    WITH fill_ids AS (
        SELECT COALESCE(array_agg(n ->> 'id'), ARRAY[]::text[]) AS ids
        FROM jsonb_array_elements(graph -> 'nodes') AS n
        WHERE n ->> 'kind' = 'fill'
    )
    SELECT jsonb_set(
        jsonb_set(
            graph,
            '{nodes}',
            COALESCE(
                (SELECT jsonb_agg(n ORDER BY ord)
                   FROM jsonb_array_elements(graph -> 'nodes') WITH ORDINALITY AS t(n, ord)
                  WHERE n ->> 'kind' IS DISTINCT FROM 'fill'),
                '[]'::jsonb)
        ),
        '{edges}',
        COALESCE(
            (SELECT jsonb_agg(e ORDER BY ord)
               FROM jsonb_array_elements(graph -> 'edges') WITH ORDINALITY AS t(e, ord), fill_ids
              WHERE NOT (COALESCE(e ->> 'source' = ANY (fill_ids.ids), false)
                      OR COALESCE(e ->> 'target' = ANY (fill_ids.ids), false))),
            '[]'::jsonb)
    )
    FROM fill_ids;
$$;

UPDATE workflows
   SET graph = pg_temp.strip_fill_nodes(graph)
 WHERE EXISTS (SELECT 1 FROM jsonb_array_elements(graph -> 'nodes') AS n WHERE n ->> 'kind' = 'fill');

UPDATE runs
   SET graph_snapshot = pg_temp.strip_fill_nodes(graph_snapshot)
 WHERE EXISTS (SELECT 1 FROM jsonb_array_elements(graph_snapshot -> 'nodes') AS n WHERE n ->> 'kind' = 'fill');

DELETE FROM node_runs WHERE kind = 'fill';
