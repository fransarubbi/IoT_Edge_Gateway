//! Repositorio central de acceso a la base de datos.
//!
//! Este módulo implementa el patrón *Repository* para encapsular
//! toda la interacción con SQLite.
//!
//! Actúa como una fachada sobre las distintas tablas del sistema,
//! delegando las operaciones concretas a los módulos especializados
//! de cada entidad (`measurement`, `monitor`, `alert_*`).
//!
//! ## Responsabilidades
//!
//! - Inicializar y configurar la base de datos
//! - Gestionar el pool de conexiones SQLite
//! - Insertar datos de forma polimórfica
//! - Extraer batches de datos pendientes
//! - Proveer consultas de estado (`has_data`)
//! - Administrar la configuración de redes (`Network`)
//!
//! ## Diseño
//!
//! - Seguro para concurrencia (usa `SqlitePool`)
//! - Pensado para ejecución como servicio `systemd`
//! - Soporta reintentos infinitos ante fallos de inicialización
//!
//! ## Límites del módulo
//!
//! - No contiene lógica de negocio
//! - No decide cuándo enviar datos al servidor
//! - No maneja reintentos de red
//!
//! ## Uso
//!
//! Este módulo es utilizado por:
//! - `dba_task`
//! - `dba_insert_task`
//! - `dba_get_task`


use std::time::Duration;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{FromRow, SqlitePool};
use crate::config::sqlite::{LIMIT, WAIT_FOR};
use crate::database::domain::{Table, TableDataVector, TableDataVectorTypes};
use crate::database::tables::alert_air::{create_table_alert_air, insert_alert_air, pop_batch_alert_air};
use crate::database::tables::alert_temp::{create_table_alert_temp, insert_alert_temp, pop_batch_alert_temp};
use crate::database::tables::measurement::{create_table_measurement, insert_measurement, pop_batch_measurement};
use crate::database::tables::monitor::{create_table_monitor, insert_monitor, pop_batch_monitor};
use crate::database::tables::network::{create_table_network, delete_network_database, get_all_network_data, insert_network_database};
use crate::network::domain::{Network, NetworkRow};


/// Repositorio central de acceso a la base de datos.
///
/// # Responsabilidad
///
/// Este struct encapsula el acceso a SQLite y actúa como una **fachada**
/// sobre las distintas tablas del sistema.
///
/// El `Repository`:
/// - inicializa la base de datos,
/// - configura parámetros de rendimiento,
/// - delega operaciones CRUD a los módulos de cada tabla,
/// - provee una API unificada para el resto del sistema.
///
/// # Diseño
///
/// - Contiene un [`SqlitePool`] compartido.
/// - Es seguro para uso concurrente.
/// - No expone detalles SQL a capas superiores.
///
#[derive(Clone, Debug)]
pub struct Repository {
    pool: SqlitePool,
}

impl Repository {

    /// Crea una nueva instancia del repositorio.
    ///
    /// # Flujo de inicialización
    ///
    /// 1. Crea el pool de conexiones SQLite.
    /// 2. Configura los parámetros de la base de datos (PRAGMAs).
    /// 3. Inicializa el esquema completo (todas las tablas).
    ///
    /// # Errores
    ///
    /// Retorna [`sqlx::Error`] si falla cualquiera de los pasos anteriores.
    pub async fn new(path: &str) -> Result<Self, sqlx::Error> {
        let pool = create_pool(path).await?;
        configure_db(&pool).await?;
        init_schema(&pool).await?;
        Ok(Self { pool })
    }

    /// Crea el repositorio con reintentos infinitos.
    ///
    /// # Motivación
    ///
    /// Esta función está pensada para servicios `systemd`:
    /// si la base de datos no está disponible al arranque,
    /// el sistema **no falla**, sino que reintenta indefinidamente.
    ///
    /// # Comportamiento
    ///
    /// - Reintenta cada `WAIT_FOR` segundos.
    /// - Loguea errores en cada intento fallido.
    /// - Retorna solo cuando la inicialización es exitosa.
    pub async fn create_repository(path: &str) -> Self {
        loop {
            match Self::new(path).await {
                Ok(repo) => return repo,
                Err(e) => {
                    log::error!("Error inicializando repo: {:?}", e);
                    tokio::time::sleep(Duration::from_secs(WAIT_FOR)).await;
                }
            }
        }
    }

    /// Inserta datos en la base de datos de forma polimórfica.
    ///
    /// # Descripción
    ///
    /// Recibe un vector de [`TableDataVector`], donde cada variante
    /// representa un lote de datos de una tabla específica.
    ///
    /// La función delega la inserción a la función correspondiente
    /// de cada tabla.
    ///
    /// # Notas de diseño
    ///
    /// - El orden de inserción respeta el orden del vector recibido.
    /// - Cada tabla maneja su propia transacción o inserción atómica.
    /// - Si ocurre un error, la operación se aborta y retorna el error.
    pub async fn insert(&self, tdv: Vec<TableDataVectorTypes>) -> Result<(), sqlx::Error> {
        for v in tdv {
            match v {
                TableDataVectorTypes::Measurement(measurement) => {
                    insert_measurement(&self.pool, measurement).await?;
                },
                TableDataVectorTypes::Monitor(monitor) => {
                    insert_monitor(&self.pool, monitor).await?;
                },
                TableDataVectorTypes::AlertTemp(alert_temp) => {
                    insert_alert_temp(&self.pool, alert_temp).await?;
                },
                TableDataVectorTypes::AlertAir(alert_air) => {
                    insert_alert_air(&self.pool, alert_air).await?;
                },
            }
        }
        Ok(())
    }

    /// Extrae y elimina un batch de datos de una tabla específica.
    ///
    /// # Descripción
    ///
    /// Según el valor de [`Table`], invoca la función `pop_batch_*`
    /// correspondiente y retorna los datos encapsulados en
    /// [`TableDataVector`].
    ///
    /// # Uso típico
    ///
    /// - Reenvío de datos pendientes al servidor.
    /// - Vaciado progresivo de la base local.
    pub async fn pop_batch(&self, table: Table, topic: &str) -> Result<TableDataVector, sqlx::Error> {

        let tdv_type = match table {
            Table::Measurement => {
                let data = pop_batch_measurement(&self.pool, topic).await?;
                TableDataVectorTypes::Measurement(data)
            },
            Table::Monitor => {
                let data = pop_batch_monitor(&self.pool, topic).await?;
                TableDataVectorTypes::Monitor(data)
            },
            Table::AlertTemp => {
                let data = pop_batch_alert_temp(&self.pool, topic).await?;
                TableDataVectorTypes::AlertTemp(data)
            },
            Table::AlertAir => {
                let data = pop_batch_alert_air(&self.pool, topic).await?;
                TableDataVectorTypes::AlertAir(data)
            },

            _ => {
                return Err(sqlx::Error::Protocol("Tipo de tabla no soportado o inválido".into()));
            }
        };

        Ok(TableDataVector::new(topic.to_string(), tdv_type))
    }

    /// Indica si una tabla contiene al menos un registro.
    ///
    /// # Uso
    ///
    /// Esta función se utiliza para:
    /// - detectar datos pendientes,
    /// - sincronizar con el servidor,
    /// - controlar flujos de reenvío.
    pub async fn has_data(&self, table: Table) -> Result<bool, sqlx::Error> {
        match table {
            Table::Measurement | Table::Monitor | Table::AlertTemp | Table::AlertAir => {
                table_has_data(&self.pool, table.table_name()).await
            },
            _ => {
                Err(sqlx::Error::Protocol("❌ Error: Tipo de tabla no soportado o inválido".into()))
            }
        }
    }

    // --- Métodos de Gestión de Redes (Network) ---

    /// Inserta una nueva configuración de red en la base de datos.
    ///
    /// Utilizado al recibir nuevas configuraciones desde el Servidor.
    pub async fn insert_network(&self, data: Network) -> Result<(), sqlx::Error> {
        insert_network_database(&self.pool, data).await?;
        Ok(())
    }

    /// Elimina una red de la base de datos por su ID.
    ///
    /// # Argumentos
    ///
    /// * `id` - El identificador único de la red (ej. "sala7").
    pub async fn delete_network(&self, id: &str) -> Result<(), sqlx::Error> {
        delete_network_database(&self.pool, id).await?;
        Ok(())
    }

    /// Obtiene todas las redes configuradas en el sistema.
    ///
    /// # Retorno
    ///
    /// Devuelve un vector de objetos [`Network`] mapeados desde la base de datos.
    /// Se utiliza para poblar la memoria (`NetworkManager`) al iniciar el sistema.
    pub async fn get_all_network(&self) -> Result<Vec<NetworkRow>, sqlx::Error> {
        let rows = get_all_network_data(&self.pool).await?;
        Ok(rows)
    }
}


/// Crea el pool de conexiones SQLite.
///
/// # Notas
///
/// - `max_connections` limita la concurrencia.
/// - SQLite maneja internamente el locking.
async fn create_pool(db_path: &str) -> Result<SqlitePool, sqlx::Error> {
    let database_url = format!("sqlite://{}", db_path);

    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect(&database_url)
        .await?;

    Ok(pool)
}


/// Configura parámetros de rendimiento y concurrencia de SQLite.
///
/// # PRAGMAs utilizados
///
/// - `journal_mode = WAL`: mejora concurrencia entre lecturas y escrituras.
/// - `synchronous = NORMAL`: balance entre seguridad y rendimiento.
/// - `busy_timeout`: espera antes de fallar por bloqueo (5000 ms).
async fn configure_db(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("PRAGMA journal_mode = WAL;").execute(pool).await?;
    sqlx::query("PRAGMA synchronous = NORMAL;").execute(pool).await?;
    sqlx::query("PRAGMA busy_timeout = 5000;").execute(pool).await?;
    Ok(())
}


/// Inicializa el esquema completo de la base de datos.
///
/// # Descripción
///
/// Crea todas las tablas necesarias para el funcionamiento del sistema.
/// Es segura de ejecutar múltiples veces (`CREATE TABLE IF NOT EXISTS`).
async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    create_table_measurement(pool).await?;
    create_table_monitor(pool).await?;
    create_table_alert_temp(pool).await?;
    create_table_alert_air(pool).await?;
    create_table_network(pool).await?;
    Ok(())
}


/// Función genérica para extraer y eliminar batches de una tabla.
///
/// # Descripción
///
/// Ejecuta un `DELETE ... RETURNING *` sobre la tabla indicada,
/// devolviendo los registros eliminados como un vector del tipo `T`.
///
/// # Seguridad
///
/// El nombre de la tabla se inserta con comillas dobles para evitar
/// problemas con palabras reservadas, pero **debe ser confiable**
/// (no provenir de input externo no validado).
pub async fn pop_batch_generic<T>(pool: &SqlitePool,
                                  table: &str,
                                  topic: &str
) -> Result<Vec<T>, sqlx::Error>
where
    T: for<'r> FromRow<'r, sqlx::sqlite::SqliteRow>
    + Send
    + Unpin
    + 'static,
{
    let sql = format!(
        r#"
        DELETE FROM "{table}"
        WHERE id IN (
            SELECT id FROM "{table}"
            WHERE topic_where_arrive = ?
            LIMIT ?
        )
        RETURNING *
        "#
    );

    let result = sqlx::query_as::<_, T>(&sql)
        .bind(topic)
        .bind(LIMIT)
        .fetch_all(pool)
        .await?;

    Ok(result)
}


/// Indica si una tabla contiene al menos un registro.
///
/// # Implementación
///
/// Utiliza una consulta mínima (`SELECT 1 LIMIT 1`)
/// para reducir carga de I/O y latencia.
async fn table_has_data(pool: &SqlitePool, table: &str) -> Result<bool, sqlx::Error> {
    // Usamos comillas dobles por seguridad
    let sql = format!("SELECT 1 FROM \"{table}\" LIMIT 1");
    let row = sqlx::query(&sql).fetch_optional(pool).await?;
    Ok(row.is_some())
}