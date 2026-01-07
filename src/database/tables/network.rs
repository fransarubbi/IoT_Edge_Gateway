use sqlx::{Executor, SqlitePool};
use crate::network::domain::{Network, NetworkRow};


/// Crea la tabla `network` en la base de datos si aún no existe.
///
/// # Descripción
///
/// Esta función inicializa el esquema de persistencia para los datos de tipo
/// [`Network`]. Se utiliza durante la fase de arranque del sistema, típicamente
/// desde la inicialización del [`Repository`].
///
/// La tabla almacena la configuración de las redes IoT conocidas por el Edge.
///
/// # Esquema de la tabla
///
/// La tabla `network` contiene las siguientes columnas:
///
/// - `id_network`: Identificador único (Primary Key, ejemplo: "sala7").
/// - `name_network`: Nombre descriptivo de la red.
/// - `topic_data`: Tópico para recepción de datos de sensores.
/// - `topic_data_qos`: Calidad de servicio para data.
/// - `topic_alert_air`: Tópico para alertas de calidad de aire.
/// - `topic_alert_air_qos`: QoS para alertas de aire.
/// - `topic_alert_temp`: Tópico para alertas de temperatura.
/// - `topic_alert_temp_qos`: QoS para alertas de temperatura.
/// - `topic_monitor`: Tópico para datos de monitoreo de estado.
/// - `topic_monitor_qos`: QoS para monitoreo.
/// - `topic_settings_hub`: Tópico de configuración proveniente del Hub.
/// - `topic_settings_qos_hub`: QoS para settings del Hub.
/// - `topic_settings_ser`: Tópico de configuración proveniente del Servidor.
/// - `topic_settings_qos_ser`: QoS para settings del Servidor.
/// - `topic_active`: Tópico para emitir el "latido" o estado activo.
/// - `topic_active_qos`: QoS para active.
/// - `active`: Flag booleano para habilitar/deshabilitar la red lógicamente.
///
/// # Errores
///
/// Retorna un [`sqlx::Error`] si:
/// - La conexión al pool falla.
/// - La sentencia SQL es inválida o no puede ejecutarse.
///
/// # Notas de diseño
///
/// - La función es *idempotente* (`IF NOT EXISTS`).
/// - No realiza migraciones: si el esquema cambia en el código, se debe
///   actualizar la base de datos manualmente o borrar el archivo `.db`.
pub async fn create_table_network(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS network (
            id_network               TEXT PRIMARY KEY,
            name_network             TEXT NOT NULL,
            topic_data               TEXT NOT NULL,
            topic_data_qos           INTEGER NOT NULL,
            topic_alert_air          TEXT NOT NULL,
            topic_alert_air_qos      INTEGER NOT NULL,
            topic_alert_temp         TEXT NOT NULL,
            topic_alert_temp_qos     INTEGER NOT NULL,
            topic_monitor            TEXT NOT NULL,
            topic_monitor_qos        INTEGER NOT NULL,
            topic_settings_hub       TEXT NOT NULL,
            topic_settings_qos_hub   INTEGER NOT NULL,
            topic_settings_ser       TEXT NOT NULL,
            topic_settings_qos_ser   INTEGER NOT NULL,
            topic_active             TEXT NOT NULL,
            topic_active_qos         INTEGER NOT NULL,
            active                   BOOLEAN NOT NULL DEFAULT TRUE
        );
        "#
    )
        .await?;
    Ok(())
}


/// Inserta una nueva configuración de red en la tabla `network`.
///
/// # Descripción
///
/// Recibe un objeto de dominio [`Network`], extrae sus campos y los persiste
/// mediante una operación atómica.
///
/// # Parámetros
///
/// - `pool`: Pool de conexiones SQLite compartido.
/// - `data`: Estructura [`Network`] con la información a persistir.
///
/// # Errores
///
/// Retorna un [`sqlx::Error`] si:
/// - Ocurre una violación de la *Primary Key* (el `id_network` ya existe).
/// - Falla la conexión o la ejecución de la query.
///
/// # Notas
///
/// - Los campos booleanos se convierten automáticamente a enteros (0/1) por SQLite.
/// - Los campos de tipo struct interno (`Topic`) se aplanan en columnas individuales.
pub async fn insert_network(
    pool: &SqlitePool,
    data: Network
) -> Result<(), sqlx::Error> {

    sqlx::query(
        r#"
        INSERT INTO network (
            id_network, name_network, topic_data, topic_data_qos,
            topic_alert_air, topic_alert_air_qos, topic_alert_temp, topic_alert_temp_qos,
            topic_monitor, topic_monitor_qos,
            topic_settings_hub, topic_settings_qos_hub,
            topic_settings_ser, topic_settings_qos_ser,
            topic_active, topic_active_qos, active
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#
    )
        .bind(data.id_network)
        .bind(data.name_network)
        .bind(data.topic_data.topic)
        .bind(data.topic_data.qos)
        .bind(data.topic_alert_air.topic)
        .bind(data.topic_alert_air.qos)
        .bind(data.topic_alert_temp.topic)
        .bind(data.topic_alert_temp.qos)
        .bind(data.topic_monitor.topic)
        .bind(data.topic_monitor.qos)
        .bind(data.topic_settings_from_hub.topic)
        .bind(data.topic_settings_from_hub.qos)
        .bind(data.topic_settings_from_ser.topic)
        .bind(data.topic_settings_from_ser.qos)
        .bind(data.topic_active.topic)
        .bind(data.topic_active.qos)
        .bind(data.active)
        .execute(pool)
        .await?;

    Ok(())
}


/// Elimina una red de la base de datos.
///
/// # Descripción
///
/// Borra la fila correspondiente al `id` proporcionado.
///
/// # Parámetros
///
/// * `id` - El identificador único de la red (ej. "sala7").
///
/// # Errores
///
/// Retorna un [`sqlx::Error`] si falla la ejecución SQL.
///
/// # Notas
///
/// Si el ID no existe, la operación es exitosa pero no borra nada
/// (rows affected = 0). No retorna error en ese caso.
pub async fn delete_network(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        DELETE FROM network
        WHERE id_network = ?
        "#
    )
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}


/// Recupera todas las redes almacenadas en la base de datos.
///
/// # Descripción
///
/// Ejecuta un `SELECT *` sobre la tabla `network` y mapea cada fila
/// a una estructura [`NetworkRow`].
///
/// # Uso
///
/// Esta función es fundamental durante el inicio del sistema para poblar
/// el `NetworkManager` en memoria con la configuración persistida.
///
/// # Errores
///
/// Retorna [`sqlx::Error`] si:
/// - El esquema de la base de datos no coincide con [`NetworkRow`].
/// - Falla la conexión.
pub async fn get_all_network_data(pool: &SqlitePool) -> Result<Vec<NetworkRow>, sqlx::Error> {
    let result = sqlx::query_as::<_, NetworkRow>(
        r#"
        SELECT * FROM network
        "#
    )
        .fetch_all(pool)
        .await?;

    Ok(result)
}