use std::{collections::HashMap, str::FromStr};

#[derive(Debug, Clone)]
pub struct AuthUser {
    uname: String,
    passwd: String,
}

impl FromStr for AuthUser {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((uname, passwd)) = s.split_once(':') {
            Ok(AuthUser {
                uname: uname.to_string(),
                passwd: passwd.to_string(),
            })
        } else {
            Err(format!("invalid username/password: {}", s))
        }
    }
}

#[derive(Debug, Clone)]
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
