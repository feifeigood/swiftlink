use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct AuthUser {
    uname: String,
    passwd: String,
}

pub struct Authenticator {
    storage: HashMap<String, String>,
}

impl Authenticator {
    pub fn new(users: Vec<AuthUser>) -> Self {
        let mut storage = HashMap::<String, String>::new();
        users.into_iter().for_each(|au| {
            _ = storage.insert(au.uname, au.passwd);
        });
        Authenticator { storage }
    }

    pub fn verify(&self, user: &str, pass: &str) -> bool {
        self.storage.get(user).is_some_and(|real| real.eq(pass))
    }
}
