ALTER TABLE node_runs ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;  -- ExecutionPlan 순서(0부터)
