use sqlx::{Executor, SqlitePool};
use crate::database::repository::pop_batch_generic;
use crate::message::domain_for_table::MonitorRow;


/// Crea la tabla `monitor` en la base de datos si aún no existe.
///
/// # Descripción
///
/// Esta función inicializa el esquema de persistencia para los datos de tipo
/// [`Monitor`].
/// Se utiliza durante la fase de arranque del sistema, típicamente desde
/// la inicialización del [`Repository`].
///
/// La tabla almacena mediciones provenientes de nodos IoT y está diseñada
/// para escritura frecuente y lectura por lotes (*batch consumption*).
///
/// # Esquema de la tabla
///
/// La tabla `monitor` contiene las siguientes columnas:
///
/// - `id`: clave primaria autoincremental.
/// - `sender_user_id`: identificador del nodo emisor.
/// - `destination_type`: tipo de destino lógico del mensaje.
/// - `destination_id`: identificador del destino.
/// - `timestamp`: instante de generación de la medición (formato texto).
///  - `topic_where_arrive`: tópico donde se recibió el mensaje.
/// - `mem_free`: memoria RAM libre total del nodo.
/// - `mem_free_hm`: heap libre mínimo.
/// - `mem_free_block`: bloque de memoria más grande.
/// - `mem_free_internal`: heap libre.
/// - `stack_free_min_coll`: mínimo stack libre de la tarea collector.
/// - `stack_free_min_pub`: mínimo stack libre de la tarea publisher.
/// - `stack_free_min_mic`: mínimo stack libre de la tarea mic.
/// - `stack_free_min_th`: mínimo stack libre de la tarea temperature.
/// - `stack_free_min_air`: mínimo stack libre de la tarea air.
/// - `stack_free_min_mon`: mínimo stack libre de la tarea monitor.
/// - `wifi_ssid`: ssid del wifi al que está conectado el nodo.
/// - `wifi_rssi`: valor rssi del wifi al que está conectado el nodo.
/// - `active_time`: tiempo activo del nodo.
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

pub async fn create_table_monitor(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS monitor (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL,
            destination_type     TEXT NOT NULL,
            destination_id       TEXT NOT NULL,
            timestamp            TEXT NOT NULL,
            topic_where_arrive   TEXT NOT NULL,
            mem_free             INTEGER NOT NULL,
            mem_free_hm          INTEGER NOT NULL,
            mem_free_block       INTEGER NOT NULL,
            mem_free_internal    INTEGER NOT NULL,
            stack_free_min_coll  INTEGER NOT NULL,
            stack_free_min_pub   INTEGER NOT NULL,
            stack_free_min_mic   INTEGER NOT NULL,
            stack_free_min_th    INTEGER NOT NULL,
            stack_free_min_air   INTEGER NOT NULL,
            stack_free_min_mon   INTEGER NOT NULL,
            wifi_ssid            TEXT NOT NULL,
            wifi_rssi            INTEGER NOT NULL,
            active_time TEXT NOT NULL
        );
        "#
    )
        .await?;

    Ok(())
}


/// Inserta un lote (*batch*) de mediciones en la tabla `monitor`.
///
/// # Descripción
///
/// Recibe un vector de [`Monitor`] y persiste todos los elementos
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

pub async fn insert_monitor(pool: &SqlitePool,
                            data_vec: Vec<MonitorRow>
                            ) -> Result<(), sqlx::Error> {

    let mut tx = pool.begin().await?;

    for data in data_vec {
        sqlx::query(
            r#"
        INSERT INTO monitor (sender_user_id, destination_type, destination_id, timestamp, topic_where_arrive,
                             mem_free, mem_free_hm, mem_free_block, mem_free_internal, stack_free_min_coll,
                             stack_free_min_pub, stack_free_min_mic, stack_free_min_th, stack_free_min_air,
                             stack_free_min_mon, wifi_ssid, wifi_rssi, active_time)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#
        )
            .bind(data.metadata.sender_user_id)
            .bind(data.metadata.destination_type)
            .bind(data.metadata.destination_id)
            .bind(data.metadata.timestamp)
            .bind(data.metadata.topic_where_arrive)
            .bind(data.mem_free)
            .bind(data.mem_free_hm)
            .bind(data.mem_free_block)
            .bind(data.mem_free_internal)
            .bind(data.stack_free_min_coll)
            .bind(data.stack_free_min_pub)
            .bind(data.stack_free_min_mic)
            .bind(data.stack_free_min_th)
            .bind(data.stack_free_min_air)
            .bind(data.stack_free_min_mon)
            .bind(data.wifi_ssid)
            .bind(data.wifi_rssi)
            .bind(data.active_time)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}


/// Extrae y elimina un lote de mediciones de la tabla `monitor`.
///
/// # Descripción
///
/// Esta función obtiene un conjunto de filas antiguas de la tabla
/// `monitor`, las elimina de la base de datos y las retorna como
/// un vector de [`Monitor`].
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

pub async fn pop_batch_monitor(pool: &SqlitePool, topic: &str) -> Result<Vec<MonitorRow>, sqlx::Error> {
    pop_batch_generic(pool, "monitor", topic).await
}

