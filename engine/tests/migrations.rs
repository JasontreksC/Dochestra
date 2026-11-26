use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn vector_extension_exists(pool: PgPool) {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_extension WHERE extname = 'vector'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn all_tables_exist(pool: PgPool) {
    let expected = [
        "bundle_files",
        "bundles",
        "node_runs",
        "rag_chunks",
        "rag_indexes",
        "runs",
        "workflows",
    ];
    let mut found: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    found.sort();
    assert_eq!(found, expected);
}
