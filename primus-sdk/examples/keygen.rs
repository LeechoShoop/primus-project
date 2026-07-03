use primus_sdk::crypto::keypair_from_seed;
use rand::RngCore;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() -> std::io::Result<()> {
    let secrets_dir = Path::new(".secrets");

    // 1. Создаем папку, если вдруг её нет
    if !secrets_dir.exists() {
        fs::create_dir_all(secrets_dir)?;
        println!("Создана папка .secrets");
    }

    let operator_key_path = secrets_dir.join("operator.key");

    // 2. Проверяем, не существует ли уже ключ, чтобы не перезаписать его случайно
    if operator_key_path.exists() {
        println!("Файл operator.key уже существует. Генерация отменена.");
        return Ok(());
    }

    println!("--- Генерация ключа оператора для Obsidian Nexus ---");

    // 3. Генерация энтропии
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    // 4. Используем SDK для проверки валидности (опционально)
    let keys = keypair_from_seed(seed);

    // 5. Записываем в .secrets/operator.key
    let mut file = fs::File::create(&operator_key_path)?;
    file.write_all(&seed)?;

    println!(
        "✅ Готово! Новый операторский ключ создан в {:?}",
        operator_key_path
    );
    println!("Public Key: {:x?}", &keys.pk[..16]); // Вывод начала публичного ключа для инфо

    Ok(())
}
