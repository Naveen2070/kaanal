use rand::Rng;
use rand_chacha::ChaCha8Rng;

pub const FIRST: &[&str] = &[
    "Aarav",
    "Priya",
    "Arjun",
    "Kavya",
    "Rohan",
    "Ananya",
    "Vikram",
    "Meera",
    "Karthik",
    "Divya",
    "Rahul",
    "Sneha",
    "Aditya",
    "Lakshmi",
    "Siddharth",
    "Pooja",
    "Naveen",
    "Deepa",
    "Varun",
    "Nithya",
];

pub const LAST: &[&str] = &[
    "Sharma",
    "Iyer",
    "Reddy",
    "Nair",
    "Patel",
    "Gupta",
    "Rao",
    "Menon",
    "Singh",
    "Krishnan",
    "Das",
    "Joshi",
    "Kumar",
    "Pillai",
    "Mehta",
    "Subramanian",
    "Bose",
    "Chatterjee",
    "Shetty",
    "Verma",
];

/// (city, state, pincode)
pub const CITIES: &[(&str, &str, &str)] = &[
    ("Chennai", "Tamil Nadu", "600001"),
    ("Coimbatore", "Tamil Nadu", "641001"),
    ("Madurai", "Tamil Nadu", "625001"),
    ("Bengaluru", "Karnataka", "560001"),
    ("Mumbai", "Maharashtra", "400001"),
    ("Pune", "Maharashtra", "411001"),
    ("Hyderabad", "Telangana", "500001"),
    ("Kochi", "Kerala", "682001"),
    ("Delhi", "Delhi", "110001"),
    ("Kolkata", "West Bengal", "700001"),
];

pub const STREETS: &[&str] = &[
    "MG Road",
    "Anna Salai",
    "Gandhi Nagar",
    "Nehru Street",
    "Park Avenue",
    "Lake View Road",
    "Temple Street",
    "Station Road",
];

pub const WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
    "enim",
];

pub fn pick<'a>(list: &[&'a str], rng: &mut ChaCha8Rng) -> &'a str {
    list[rng.random_range(0..list.len())]
}
