use nanoid::nanoid;

const ID_ALPHABET: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i',
    'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B',
    'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U',
    'V', 'W', 'X', 'Y', 'Z',
];

pub(crate) fn generate_sandbox_id() -> String {
    nanoid!(10, &ID_ALPHABET)
}

#[cfg(test)]
mod tests {
    use super::generate_sandbox_id;

    #[test]
    fn generate_sandbox_id_without_hyphen() {
        let id = generate_sandbox_id();
        assert!(!id.contains('-'));
    }
}
