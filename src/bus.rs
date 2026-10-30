// SPDX-License-Identifier: GPL-3.0-only

//! One connection to each bus, shared: a new one for every read would
//! connect, authenticate and say hello again each time.

use tokio::sync::OnceCell;

pub async fn system() -> zbus::Result<zbus::Connection> {
    static BUS: OnceCell<zbus::Connection> = OnceCell::const_new();
    BUS.get_or_try_init(zbus::Connection::system).await.cloned()
}

pub async fn session() -> zbus::Result<zbus::Connection> {
    static BUS: OnceCell<zbus::Connection> = OnceCell::const_new();
    BUS.get_or_try_init(zbus::Connection::session).await.cloned()
}
