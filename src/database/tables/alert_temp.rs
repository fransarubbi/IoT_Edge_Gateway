use sqlx::{Executor, SqlitePool};
use crate::database::repository::pop_batch_generic;
use crate::message::domain_for_table::AlertThRow;


/// Crea la tabla `alert_temp` en la base de datos si aún no existe.
///
/// # Descripción
///
/// Esta función inicializa el esquema de persistencia para los datos de tipo
/// [`AlertTh`].
/// Se utiliza durante la fase de arranque del sistema, típicamente desde
/// la inicialización del [`Repository`].
///
/// La tabla almacena mediciones provenientes de nodos IoT y está diseñada
/// para escritura frecuente y lectura por lotes (*batch consumption*).
///
/// # Esquema de la tabla
///
/// La tabla `alert_temp` contiene las siguientes columnas:
///
/// - `id`: clave primaria autoincremental.
/// - `sender_user_id`: identificador del nodo emisor.
/// - `destination_type`: tipo de destino lógico del mensaje.
/// - `destination_id`: identificador del destino.
/// - `timestamp`: instante de generación de la medición (formato texto).
/// - `topic_where_arrive`: tópico donde se recibió el mensaje.
/// - `initial_temp`: temperatura estable previa a la alarma.
/// - `actual_temp`: temperatura de alarma.
///
/// # Errores
///
/// Retorna un [`sqlx::Error`] si:
/// - la conexión al pool falla,
/// - la sentencia SQL no puede ejecutarse.
///
/// # Notas de diseño
///
/// - La función es *idempotente* gracias a `CREATE TABLE IF NOT EXISTS`.
/// - No realiza migraciones ni validaciones de versión del esquema.
/// - Se asume que el nombre de la tabla es estable y conocido por el sistema.
///

pub async fn create_table_alert_temp(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS alert_temp (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL,
            destination_type     TEXT NOT NULL,
            destination_id       TEXT NOT NULL,
            timestamp            TEXT NOT NULL,
            topic_where_arrive   TEXT NOT NULL,
            initial_temp         REAL NOT NULL,
            actual_temp          REAL NOT NULL
        );
        "#
    )
        .await?;
    
    Ok(())
}


/// Inserta un lote (*batch*) de mediciones en la tabla `alert_temp`.
///
/// # Descripción
///
/// Recibe un vector de [`AlertTh`] y persiste todos los elementos
/// dentro de una única transacción SQL.
///
/// Esta función está optimizada para inserciones por lotes y es utilizada
/// por tareas asincrónicas de larga duración que acumulan datos antes
/// de escribirlos en disco.
///
/// # Comportamiento transaccional
///
/// - Todas las inserciones se realizan dentro de una única transacción.
/// - Si una inserción falla, **ningún dato del batch es persistido**.
/// - La transacción se confirma solo si todas las operaciones tienen éxito.
///
/// # Parámetros
///
/// - `pool`: pool de conexiones SQLite compartido por el sistema.
/// - `data_vec`: vector de mediciones a persistir.
///
/// # Errores
///
/// Retorna un [`sqlx::Error`] si:
/// - falla el inicio de la transacción,
/// - alguna sentencia `INSERT` falla,
/// - el `commit` no puede completarse.
///
/// # Notas de diseño
///
/// - Esta función **consume** el vector recibido.
/// - No realiza validación semántica de los datos.
/// - Está pensada para ser llamada con batches relativamente pequeños
///   (controlados por `BATCH_SIZE` en capas superiores).
///

pub async fn insert_alert_temp(pool: &SqlitePool,
                               data_vec: Vec<AlertThRow>
                              ) -> Result<(), sqlx::Error> {

    let mut tx = pool.begin().await?;

    for data in data_vec {
        sqlx::query(
            r#"
        INSERT INTO alert_temp (sender_user_id, destination_type, destination_id,
                                timestamp, topic_where_arrive, initial_temp,
                                actual_temp)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#
        )
            .bind(data.metadata.sender_user_id)
            .bind(data.metadata.destination_type)
            .bind(data.metadata.destination_id)
            .bind(data.metadata.timestamp)
            .bind(data.metadata.topic_where_arrive)
            .bind(data.initial_temp)
            .bind(data.actual_temp)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}


/// Extrae y elimina un lote de mediciones de la tabla `alert_temp`.
///
/// # Descripción
///
/// Esta función obtiene un conjunto de filas antiguas de la tabla
/// `alert_temp`, las elimina de la base de datos y las retorna como
/// un vector de [`AlertTh`].
///
/// Se utiliza cuando el sistema necesita vaciar la base de datos local de forma controlada.
///
/// # Detalles de implementación
///
/// - La selección y eliminación se realizan en una única sentencia SQL
///   (`DELETE ... RETURNING *`).
/// - El tamaño del batch está definido internamente por la capa de repositorio
///   (por ejemplo, mediante `LIMIT`).
///
/// # Errores
///
/// Retorna un [`sqlx::Error`] si:
/// - falla la consulta SQL,
/// - ocurre un error de conexión con la base de datos.
///
/// # Notas de diseño
///
/// - El orden de extracción es ascendente por `id`.
/// - Si la tabla está vacía, retorna un vector vacío.
/// - La lógica específica del SQL se delega a [`pop_batch_generic`].
///

pub async fn pop_batch_alert_temp(pool: &SqlitePool, topic: &str) -> Result<Vec<AlertThRow>, sqlx::Error> {
    pop_batch_generic(pool, "alert_temp", topic).await
}

