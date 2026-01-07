use sqlx::{Executor, SqlitePool};
use crate::database::repository::pop_batch_generic;
use crate::message::domain_for_table::MeasurementRow;


/// Crea la tabla `measurement` en la base de datos si aún no existe.
///
/// # Descripción
///
/// Esta función inicializa el esquema de persistencia para los datos de tipo
/// [`Measurement`].
/// Se utiliza durante la fase de arranque del sistema, típicamente desde
/// la inicialización del [`Repository`].
///
/// La tabla almacena mediciones provenientes de nodos IoT y está diseñada
/// para escritura frecuente y lectura por lotes (*batch consumption*).
///
/// # Esquema de la tabla
///
/// La tabla `measurement` contiene las siguientes columnas:
///
/// - `id`: clave primaria autoincremental.
/// - `sender_user_id`: identificador del nodo emisor.
/// - `destination_type`: tipo de destino lógico del mensaje.
/// - `destination_id`: identificador del destino.
/// - `timestamp`: instante de generación de la medición (formato texto).
/// - `topic_where_arrive`: tópico donde se recibió el mensaje.
/// - `ipv4addr`: dirección IPv4 del nodo.
/// - `wifi_ssid`: SSID de la red WiFi.
/// - `pulse_counter`: contador de pulsos.
/// - `pulse_max_duration`: duración máxima de pulso.
/// - `temperature`: temperatura medida.
/// - `humidity`: humedad relativa.
/// - `co2_ppm`: concentración de CO₂.
/// - `sample`: número de muestra.
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

pub async fn create_table_measurement(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS measurement (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL,
            destination_type     TEXT NOT NULL,
            destination_id       TEXT NOT NULL,
            timestamp            TEXT NOT NULL,
            topic_where_arrive   TEXT NOT NULL,
            ipv4addr             TEXT NOT NULL,
            wifi_ssid            TEXT NOT NULL,
            pulse_counter        INTEGER NOT NULL,
            pulse_max_duration   INTEGER NOT NULL,
            temperature          REAL NOT NULL,
            humidity             REAL NOT NULL,
            co2_ppm              REAL NOT NULL,
            sample               INTEGER NOT NULL
        );
        "#
    )
        .await?;

    Ok(())
}


/// Inserta un lote (*batch*) de mediciones en la tabla `measurement`.
///
/// # Descripción
///
/// Recibe un vector de [`Measurement`] y persiste todos los elementos
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

pub async fn insert_measurement(pool: &SqlitePool,
                                data_vec: Vec<MeasurementRow>
                               ) -> Result<(), sqlx::Error> {

    let mut tx = pool.begin().await?;

    for data in data_vec {
        sqlx::query(
            r#"
            INSERT INTO measurement (
                sender_user_id,
                destination_type,
                destination_id,
                timestamp,
                topic_where_arrive,
                ipv4addr,
                wifi_ssid,
                pulse_counter,
                pulse_max_duration,
                temperature,
                humidity,
                co2_ppm,
                sample
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
            .bind(data.metadata.sender_user_id)
            .bind(data.metadata.destination_type)
            .bind(data.metadata.destination_id)
            .bind(data.metadata.timestamp)
            .bind(data.metadata.topic_where_arrive)
            .bind(data.ipv4addr.to_string())
            .bind(data.wifi_ssid)
            .bind(data.pulse_counter)
            .bind(data.pulse_max_duration)
            .bind(data.temperature)
            .bind(data.humidity)
            .bind(data.co2_ppm)
            .bind(data.sample)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}


/// Extrae y elimina un lote de mediciones de la tabla `measurement`.
///
/// # Descripción
///
/// Esta función obtiene un conjunto de filas antiguas de la tabla
/// `measurement`, las elimina de la base de datos y las retorna como
/// un vector de [`Measurement`].
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

pub async fn pop_batch_measurement(pool: &SqlitePool, topic: &str) -> Result<Vec<MeasurementRow>, sqlx::Error> {
    pop_batch_generic(pool, "measurement", topic).await
}

