use std::sync::{Arc, Mutex, OnceLock};

use keyring_core::{CredentialStore, Entry, Error, Result, get_default_store, set_default_store};

#[derive(Clone, Debug)]
enum CachedInitError {
    PlatformFailure(String),
    NoStorageAccess(String),
    NoDefaultStore,
    NotSupportedByStore(String),
    BadStoreFormat(String),
    Invalid { field: String, reason: String },
    Other(String),
}

impl CachedInitError {
    fn from_error(err: Error) -> Self {
        match err {
            Error::PlatformFailure(inner) => Self::PlatformFailure(inner.to_string()),
            Error::NoStorageAccess(inner) => Self::NoStorageAccess(inner.to_string()),
            Error::NoDefaultStore => Self::NoDefaultStore,
            Error::NotSupportedByStore(issue) => Self::NotSupportedByStore(issue),
            Error::BadStoreFormat(reason) => Self::BadStoreFormat(reason),
            Error::Invalid(field, reason) => Self::Invalid { field, reason },
            other => Self::Other(other.to_string()),
        }
    }

    fn to_error(&self) -> Error {
        match self {
            Self::PlatformFailure(message) => {
                Error::PlatformFailure(Box::new(std::io::Error::other(message.clone())))
            }
            Self::NoStorageAccess(message) => {
                Error::NoStorageAccess(Box::new(std::io::Error::other(message.clone())))
            }
            Self::NoDefaultStore => Error::NoDefaultStore,
            Self::NotSupportedByStore(issue) => Error::NotSupportedByStore(issue.clone()),
            Self::BadStoreFormat(reason) => Error::BadStoreFormat(reason.clone()),
            Self::Invalid { field, reason } => Error::Invalid(field.clone(), reason.clone()),
            Self::Other(message) => {
                Error::PlatformFailure(Box::new(std::io::Error::other(message.clone())))
            }
        }
    }
}

#[derive(Clone, Debug)]
enum InitState {
    Uninitialized,
    Initialized,
    Failed(CachedInitError),
}

fn init_state() -> &'static Mutex<InitState> {
    static STATE: OnceLock<Mutex<InitState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(InitState::Uninitialized))
}

pub(crate) fn entry(service: &str, user: &str) -> Result<Entry> {
    ensure_default_store()?;
    Entry::new(service, user)
}

fn ensure_default_store() -> Result<()> {
    if get_default_store().is_some() {
        return Ok(());
    }

    let mut state = match init_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    match &*state {
        InitState::Initialized => return Ok(()),
        InitState::Failed(err) => return Err(err.to_error()),
        InitState::Uninitialized => {}
    }

    if get_default_store().is_some() {
        *state = InitState::Initialized;
        return Ok(());
    }

    match build_default_store() {
        Ok(store) => {
            set_default_store(store);
            *state = InitState::Initialized;
            Ok(())
        }
        Err(err) => {
            let cached = CachedInitError::from_error(err);
            *state = InitState::Failed(cached.clone());
            Err(cached.to_error())
        }
    }
}

#[cfg(not(test))]
fn build_default_store() -> Result<Arc<CredentialStore>> {
    build_platform_store()
}

#[cfg(test)]
type StoreFactory = fn() -> Result<Arc<CredentialStore>>;

#[cfg(test)]
fn store_factory() -> &'static Mutex<StoreFactory> {
    static FACTORY: OnceLock<Mutex<StoreFactory>> = OnceLock::new();
    FACTORY.get_or_init(|| Mutex::new(build_platform_store as StoreFactory))
}

#[cfg(test)]
fn build_default_store() -> Result<Arc<CredentialStore>> {
    let factory = match store_factory().lock() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    };
    factory()
}

fn build_platform_store() -> Result<Arc<CredentialStore>> {
    #[cfg(target_os = "macos")]
    {
        let store: Arc<CredentialStore> = apple_native_keyring_store::keychain::Store::new()?;
        return Ok(store);
    }

    #[cfg(target_os = "linux")]
    {
        let store: Arc<CredentialStore> = dbus_secret_service_keyring_store::Store::new()?;
        return Ok(store);
    }

    #[allow(unreachable_code)]
    Err(Error::NotSupportedByStore(
        "OS secret storage is only supported on macOS and Linux".to_string(),
    ))
}

#[cfg(test)]
pub(crate) struct TestStoreFactoryGuard {
    previous_factory: StoreFactory,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for TestStoreFactoryGuard {
    fn drop(&mut self) {
        match store_factory().lock() {
            Ok(mut guard) => *guard = self.previous_factory,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = self.previous_factory;
            }
        }
        reset_init_state_for_tests();
    }
}

#[cfg(test)]
pub(crate) fn override_store_factory_for_tests(factory: StoreFactory) -> TestStoreFactoryGuard {
    let lock = crate::lock_test_env();
    let previous_factory = match store_factory().lock() {
        Ok(mut guard) => {
            let previous = *guard;
            *guard = factory;
            previous
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            let previous = *guard;
            *guard = factory;
            previous
        }
    };
    reset_init_state_for_tests();
    TestStoreFactoryGuard {
        previous_factory,
        _lock: lock,
    }
}

#[cfg(test)]
fn reset_init_state_for_tests() {
    match init_state().lock() {
        Ok(mut state) => *state = InitState::Uninitialized,
        Err(poisoned) => {
            let mut state = poisoned.into_inner();
            *state = InitState::Uninitialized;
        }
    }
    let _ = keyring_core::unset_default_store();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn mock_store_factory() -> Result<Arc<CredentialStore>> {
        let store: Arc<CredentialStore> = keyring_core::mock::Store::new()?;
        Ok(store)
    }

    fn failing_store_factory() -> Result<Arc<CredentialStore>> {
        Err(Error::NoStorageAccess(Box::new(std::io::Error::other(
            "secret storage unavailable",
        ))))
    }

    static FAILING_FACTORY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_failing_store_factory() -> Result<Arc<CredentialStore>> {
        FAILING_FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);
        failing_store_factory()
    }

    #[test]
    fn entry_uses_mock_store_from_test_factory() {
        let _guard = override_store_factory_for_tests(mock_store_factory);
        let entry = entry("service", "user").expect("entry");
        entry.set_password("secret").expect("set");
        assert_eq!(entry.get_password().expect("get"), "secret");
    }

    #[test]
    fn entry_propagates_store_initialization_failure() {
        let _guard = override_store_factory_for_tests(failing_store_factory);
        let err = entry("service", "user").expect_err("store init should fail");
        assert!(matches!(err, Error::NoStorageAccess(_)));
        assert!(get_default_store().is_none());
    }

    #[test]
    fn entry_reuses_cached_initialization_failure() {
        FAILING_FACTORY_CALLS.store(0, Ordering::SeqCst);
        let _guard = override_store_factory_for_tests(counting_failing_store_factory);

        let first = entry("service", "user").expect_err("first init should fail");
        let second = entry("service", "user").expect_err("cached failure should be reused");

        assert!(matches!(first, Error::NoStorageAccess(_)));
        assert!(matches!(second, Error::NoStorageAccess(_)));
        assert_eq!(FAILING_FACTORY_CALLS.load(Ordering::SeqCst), 1);
        assert!(get_default_store().is_none());
    }

    #[test]
    fn reset_init_state_recovers_from_poisoned_mutex() {
        let _lock = crate::lock_test_env();

        let _ = std::panic::catch_unwind(|| {
            let _guard = init_state().lock().expect("state lock");
            panic!("poison init state");
        });

        reset_init_state_for_tests();

        let state = match init_state().lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        assert!(matches!(state, InitState::Uninitialized));
    }

    #[test]
    fn drop_restores_factory_after_poisoned_mutex() {
        let lock = crate::lock_test_env();

        let _ = std::panic::catch_unwind(|| {
            let _guard = store_factory().lock().expect("factory lock");
            panic!("poison factory");
        });

        let previous_factory = match store_factory().lock() {
            Ok(mut guard) => {
                let previous = *guard;
                *guard = counting_failing_store_factory;
                previous
            }
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                let previous = *guard;
                *guard = counting_failing_store_factory;
                previous
            }
        };

        let guard = TestStoreFactoryGuard {
            previous_factory,
            _lock: lock,
        };
        drop(guard);

        let factory = match store_factory().lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        };

        assert!(std::ptr::fn_addr_eq(
            factory,
            build_platform_store as StoreFactory
        ));
    }
}
