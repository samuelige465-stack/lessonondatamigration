// Paste this into src/main.rs in RustRover and press Run. No dependencies needed.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// ---------------------------------------------------------------
// 1. N+1 QUERY PROBLEM
// A fake "database" that counts how many queries it receives.
// ---------------------------------------------------------------
struct Db {
    users: Vec<(u32, &'static str)>,  // (id, name)
    orders: Vec<(u32, &'static str)>, // (user_id, item)
    queries: u32,
}

impl Db {
    fn new() -> Self {
        let users = (1..=100).map(|id| (id, "user")).collect();
        let orders = (1..=100).map(|id| (id, "shoe")).collect();
        Db { users, orders, queries: 0 }
    }

    // SELECT * FROM users
    fn all_users(&mut self) -> Vec<(u32, &'static str)> {
        self.queries += 1;
        self.users.clone()
    }

    // SELECT * FROM orders WHERE user_id = ?
    fn orders_for_user(&mut self, user_id: u32) -> Vec<&'static str> {
        self.queries += 1;
        self.orders.iter().filter(|o| o.0 == user_id).map(|o| o.1).collect()
    }

    // SELECT * FROM orders WHERE user_id IN (...)   <- like a JOIN
    fn orders_for_users(&mut self, ids: &[u32]) -> HashMap<u32, Vec<&'static str>> {
        self.queries += 1;
        let mut map: HashMap<u32, Vec<&'static str>> = HashMap::new();
        for (uid, item) in &self.orders {
            if ids.contains(uid) {
                map.entry(*uid).or_default().push(*item);
            }
        }
        map
    }
}

fn demo_n_plus_one() {
    println!("--- 1. N+1 ---");

    // JUNIOR: 1 query for users, then 1 query PER user
    let mut db = Db::new();
    let users = db.all_users();
    for (id, _) in &users {
        let _orders = db.orders_for_user(*id);
    }
    println!("Junior: {} queries", db.queries); // 101

    // SENIOR: 1 query for users, 1 query for ALL their orders
    let mut db = Db::new();
    let users = db.all_users();
    let ids: Vec<u32> = users.iter().map(|(id, _)| *id).collect();
    let _orders = db.orders_for_users(&ids);
    println!("Senior: {} queries", db.queries); // 2
}

// ---------------------------------------------------------------
// 2. SOFT DELETE vs HARD DELETE
// ---------------------------------------------------------------
struct Row {
    id: u32,
    name: String,
    deleted_at: Option<u64>, // None = alive, Some(time) = soft deleted
}

// Every "query" must filter out deleted rows
fn visible(rows: &[Row]) -> Vec<&str> {
    rows.iter()
        .filter(|r| r.deleted_at.is_none())
        .map(|r| r.name.as_str())
        .collect()
}

fn demo_soft_delete() {
    println!("\n--- 2. Soft delete ---");
    let mut rows = vec![
        Row { id: 1, name: "Ada".to_string(), deleted_at: None },
        Row { id: 2, name: "Bola".to_string(), deleted_at: None },
    ];

    rows[0].deleted_at = Some(1_700_000_000); // soft delete: just set the date
    println!("After soft delete: {:?}", visible(&rows));

    rows[0].deleted_at = None; // client asks to restore, easy!
    println!("After restore:     {:?}", visible(&rows));

    rows.retain(|r| r.id != 2); // hard delete: gone forever
    println!("After hard delete: {:?}", visible(&rows));
}

// ---------------------------------------------------------------
// 3. SAFE MIGRATION: nullable -> backfill -> required
// ---------------------------------------------------------------
struct UserOld {
    id: u32,
    name: String,
    email: Option<String>, // STEP 1: new column, nullable
}

struct UserNew {
    id: u32,
    name: String,
    email: String, // STEP 3: now required
}

fn demo_migration() {
    println!("\n--- 3. Migration ---");

    // Existing production data. The new column exists but is empty.
    let mut users = vec![
        UserOld { id: 1, name: "Ada".to_string(), email: None },
        UserOld { id: 2, name: "Bola".to_string(), email: None },
    ];

    // STEP 2: backfill old rows
    for u in users.iter_mut() {
        if u.email.is_none() {
            u.email = Some(format!("{}@example.com", u.name.to_lowercase()));
        }
    }

    // STEP 3: make it required (fails loudly if the backfill missed a row)
    let users: Vec<UserNew> = users
        .into_iter()
        .map(|u| UserNew {
            id: u.id,
            name: u.name,
            email: u.email.expect("backfill missed a row!"),
        })
        .collect();

    for u in &users {
        println!("{} {} {}", u.id, u.name, u.email);
    }
}

// ---------------------------------------------------------------
// 4. LOCKING: two buyers, one item left
// ---------------------------------------------------------------

// PESSIMISTIC: lock first, so the second buyer must wait
fn demo_pessimistic() {
    println!("\n--- 4a. Pessimistic lock ---");
    let stock = Arc::new(Mutex::new(1u32));

    let handles: Vec<_> = (1..=2)
        .map(|buyer| {
            let stock = Arc::clone(&stock);
            thread::spawn(move || {
                let mut s = stock.lock().unwrap(); // like SELECT ... FOR UPDATE
                if *s > 0 {
                    thread::sleep(Duration::from_millis(50)); // slow work
                    *s -= 1;
                    println!("buyer {buyer}: bought it!");
                } else {
                    println!("buyer {buyer}: sold out");
                }
            }) // lock released here
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

// OPTIMISTIC: no lock while working, check the version before saving
struct Item {
    stock: u32,
    version: u32,
}

fn demo_optimistic() {
    println!("\n--- 4b. Optimistic lock ---");
    let item = Arc::new(Mutex::new(Item { stock: 1, version: 0 }));

    let handles: Vec<_> = (1..=2)
        .map(|buyer| {
            let item = Arc::clone(&item);
            thread::spawn(move || loop {
                // read a snapshot, then let go of the lock
                let (stock, version) = {
                    let i = item.lock().unwrap();
                    (i.stock, i.version)
                };

                if stock == 0 {
                    println!("buyer {buyer}: sold out");
                    break;
                }

                thread::sleep(Duration::from_millis(50)); // slow work, no lock held

                let mut i = item.lock().unwrap();
                if i.version == version {
                    i.stock -= 1;
                    i.version += 1; // bump version so others know it changed
                    println!("buyer {buyer}: bought it!");
                    break;
                }
                println!("buyer {buyer}: someone else got there first, retrying...");
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

fn main() {
    demo_n_plus_one();
    demo_soft_delete();
    demo_migration();
    demo_pessimistic();
    demo_optimistic();
}