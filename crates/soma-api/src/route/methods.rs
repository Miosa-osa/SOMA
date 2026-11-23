//! Tunnel and autosize routes, and the method checks every route shares.

use super::{ApiError, InstanceId, Method, Request, Route, path_instance_id};

pub(super) fn tunnel(method: Method, instance: InstanceId, port: &str) -> Result<Route, ApiError> {
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            ApiError::new(
                400,
                "invalid_port",
                "the tunnel port must be 1 to 65535",
                false,
            )
        })?;
    match method {
        Method::Get => Ok(Route::Tunnel(instance, port)),
        _ => Err(ApiError::method_not_allowed()),
    }
}

pub(super) fn autosize(request: &Request, instance: &str) -> Result<Route, ApiError> {
    let instance_id = path_instance_id(instance)?;
    match request.method {
        Method::Get => Ok(Route::Autosize(instance_id, false)),
        Method::Put => Ok(Route::Autosize(instance_id, true)),
        _ => Err(ApiError::method_not_allowed()),
    }
}

pub(super) fn get_only(request: &Request, route: Route) -> Result<Route, ApiError> {
    if request.method == Method::Get {
        Ok(route)
    } else {
        Err(ApiError::method_not_allowed())
    }
}

pub(super) fn post_only(request: &Request, route: Route) -> Result<Route, ApiError> {
    if request.method == Method::Post {
        Ok(route)
    } else {
        Err(ApiError::method_not_allowed())
    }
}
