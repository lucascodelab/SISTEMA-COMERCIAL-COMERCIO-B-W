use chrono::Local;
use rusqlite::{backup::Backup, params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Mutex, time::Duration};
use tauri::{Manager, State};

struct Database(Mutex<Connection>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordInput { id: Option<i64>, name: String, code: Option<String>, phone: Option<String>, email: Option<String>, notes: Option<String>, category: Option<String>, supplier_id: Option<i64>, cost_cents: Option<i64>, price_cents: Option<i64>, stock: Option<i64>, minimum_stock: Option<i64>, active: Option<bool> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthResult { id: i64, name: String, username: String }

fn now() -> String { Local::now().format("%Y-%m-%d %H:%M:%S").to_string() }
fn digest(value: &str) -> String { let mut hasher = Sha256::new(); hasher.update(value.as_bytes()); hex::encode(hasher.finalize()) }

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute_batch("PRAGMA foreign_keys = ON;
    CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS company (id INTEGER PRIMARY KEY CHECK(id=1), name TEXT NOT NULL, cnpj TEXT, phone TEXT, address TEXT, updated_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS categories (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS suppliers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, contact TEXT, phone TEXT, email TEXT, notes TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, phone TEXT, email TEXT, notes TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sku TEXT NOT NULL UNIQUE, category TEXT, supplier_id INTEGER REFERENCES suppliers(id) ON DELETE SET NULL, cost_cents INTEGER NOT NULL DEFAULT 0 CHECK(cost_cents >= 0), price_cents INTEGER NOT NULL CHECK(price_cents >= 0), stock INTEGER NOT NULL DEFAULT 0 CHECK(stock >= 0), minimum_stock INTEGER NOT NULL DEFAULT 0 CHECK(minimum_stock >= 0), active INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS cash_sessions (id INTEGER PRIMARY KEY, opened_at TEXT NOT NULL, closed_at TEXT, opening_cents INTEGER NOT NULL DEFAULT 0, closing_cents INTEGER, status TEXT NOT NULL CHECK(status IN ('OPEN','CLOSED')));
    CREATE UNIQUE INDEX IF NOT EXISTS only_one_open_cash ON cash_sessions(status) WHERE status='OPEN';
    CREATE TABLE IF NOT EXISTS sales (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id) ON DELETE SET NULL, cash_session_id INTEGER REFERENCES cash_sessions(id), total_cents INTEGER NOT NULL CHECK(total_cents >= 0), discount_cents INTEGER NOT NULL DEFAULT 0, payment_method TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'COMPLETED', created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS sale_items (id INTEGER PRIMARY KEY, sale_id INTEGER NOT NULL REFERENCES sales(id) ON DELETE CASCADE, product_id INTEGER NOT NULL REFERENCES products(id), quantity INTEGER NOT NULL CHECK(quantity > 0), unit_price_cents INTEGER NOT NULL, total_cents INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS stock_movements (id INTEGER PRIMARY KEY, product_id INTEGER NOT NULL REFERENCES products(id), type TEXT NOT NULL CHECK(type IN ('IN','OUT','ADJUSTMENT','SALE')), quantity INTEGER NOT NULL, reason TEXT, created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS cash_movements (id INTEGER PRIMARY KEY, cash_session_id INTEGER NOT NULL REFERENCES cash_sessions(id), type TEXT NOT NULL CHECK(type IN ('IN','OUT','SALE')), amount_cents INTEGER NOT NULL, description TEXT NOT NULL, created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS accounts (id INTEGER PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('PAYABLE','RECEIVABLE')), description TEXT NOT NULL, person_name TEXT, due_date TEXT NOT NULL, amount_cents INTEGER NOT NULL CHECK(amount_cents >= 0), status TEXT NOT NULL DEFAULT 'PENDING', paid_at TEXT, created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS idx_products_name ON products(name); CREATE INDEX IF NOT EXISTS idx_sales_created ON sales(created_at); CREATE INDEX IF NOT EXISTS idx_accounts_due ON accounts(due_date);")?;
  let count: i64 = conn.query_row("SELECT count(*) FROM users", [], |r| r.get(0))?;
  if count == 0 { let t=now(); conn.execute("INSERT INTO users(name,username,password_hash,created_at) VALUES(?1,?2,?3,?4)", params!["Administrador", "admin", digest("admin123"), t])?; conn.execute("INSERT INTO company(id,name,updated_at) VALUES(1,?1,?2)", params!["Meu Comércio", now()])?; }
  conn.execute("INSERT OR IGNORE INTO settings(key,value,updated_at) VALUES('currency','BRL',?1),('date_format','DD/MM/AAAA',?1),('appearance','dark',?1)",params![now()])?;
  let _ = conn.execute("ALTER TABLE accounts ADD COLUMN category TEXT", []);
  let _ = conn.execute("ALTER TABLE company ADD COLUMN logo_path TEXT", []);
  Ok(())
}

fn rows(conn: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Value>, String> {
  let mut statement=conn.prepare(sql).map_err(|e|e.to_string())?; let names=statement.column_names().iter().map(|s|s.to_string()).collect::<Vec<_>>();
  let iter=statement.query_map(args, |row| { let mut result=serde_json::Map::new(); for (i,n) in names.iter().enumerate() { let value: rusqlite::types::Value=row.get(i)?; result.insert(n.clone(), match value { rusqlite::types::Value::Null=>Value::Null, rusqlite::types::Value::Integer(v)=>json!(v), rusqlite::types::Value::Real(v)=>json!(v), rusqlite::types::Value::Text(v)=>json!(v), rusqlite::types::Value::Blob(_)=>Value::Null }); } Ok(Value::Object(result)) }).map_err(|e|e.to_string())?;
  iter.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
}

#[tauri::command]
fn login(db: State<Database>, username: String, password: String) -> Result<AuthResult,String> { let conn=db.0.lock().map_err(|_|"Banco indisponível")?; conn.query_row("SELECT id,name,username FROM users WHERE username=?1 AND password_hash=?2 AND active=1",params![username.trim(),digest(&password)],|r|Ok(AuthResult{id:r.get(0)?,name:r.get(1)?,username:r.get(2)?})).map_err(|_|"Usuário ou senha inválidos.".into()) }

#[tauri::command]
fn list_records(db: State<Database>, entity: String, search: Option<String>) -> Result<Vec<Value>,String> { let conn=db.0.lock().map_err(|_|"Banco indisponível")?; let term=format!("%{}%",search.unwrap_or_default()); match entity.as_str() {
  "products"=>rows(&conn,"SELECT p.*, COALESCE(s.name,'—') supplier_name FROM products p LEFT JOIN suppliers s ON s.id=p.supplier_id WHERE p.name LIKE ?1 OR p.sku LIKE ?1 ORDER BY p.name",&[&term]),
  "customers"=>rows(&conn,"SELECT c.*,COALESCE((SELECT SUM(s.total_cents) FROM sales s WHERE s.customer_id=c.id),0) total_purchases,(SELECT MAX(created_at) FROM sales s WHERE s.customer_id=c.id) last_purchase FROM customers c WHERE c.name LIKE ?1 OR COALESCE(c.phone,'') LIKE ?1 ORDER BY c.name",&[&term]),
  "suppliers"=>rows(&conn,"SELECT * FROM suppliers WHERE name LIKE ?1 OR COALESCE(contact,'') LIKE ?1 ORDER BY name",&[&term]),
  "stock"=>rows(&conn,"SELECT sm.*,p.name product_name,p.sku FROM stock_movements sm JOIN products p ON p.id=sm.product_id ORDER BY sm.id DESC LIMIT 200",&[]),
  "sales"=>rows(&conn,"SELECT s.*,COALESCE(c.name,'Consumidor final') customer_name FROM sales s LEFT JOIN customers c ON c.id=s.customer_id ORDER BY s.id DESC LIMIT 200",&[]),
  "accounts"=>rows(&conn,"SELECT * FROM accounts WHERE description LIKE ?1 OR COALESCE(person_name,'') LIKE ?1 ORDER BY due_date",&[&term]),
  "cash"=>rows(&conn,"SELECT cm.*,cs.status FROM cash_movements cm JOIN cash_sessions cs ON cs.id=cm.cash_session_id ORDER BY cm.id DESC LIMIT 200",&[]),
  _=>Err("Módulo inválido".into()) }
}

#[tauri::command]
fn save_record(db: State<Database>, entity: String, input: RecordInput) -> Result<i64,String> { let conn=db.0.lock().map_err(|_|"Banco indisponível")?; if input.name.trim().is_empty(){return Err("Informe o nome.".into())}; let t=now(); let active=if input.active.unwrap_or(true){1}else{0}; match entity.as_str(){
 "products"=>{let sku=input.code.unwrap_or_default();if sku.trim().is_empty(){return Err("Informe o SKU.".into())};let cost=input.cost_cents.unwrap_or(0);let price=input.price_cents.unwrap_or(0);if price<0||cost<0{return Err("Valores inválidos.".into())};if let Some(id)=input.id{conn.execute("UPDATE products SET name=?1,sku=?2,category=?3,supplier_id=?4,cost_cents=?5,price_cents=?6,stock=?7,minimum_stock=?8,active=?9,updated_at=?10 WHERE id=?11",params![input.name,sku,input.category,input.supplier_id,cost,price,input.stock.unwrap_or(0),input.minimum_stock.unwrap_or(0),active,t,id]).map_err(|e|e.to_string())?;Ok(id)}else{conn.execute("INSERT INTO products(name,sku,category,supplier_id,cost_cents,price_cents,stock,minimum_stock,active,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",params![input.name,sku,input.category,input.supplier_id,cost,price,input.stock.unwrap_or(0),input.minimum_stock.unwrap_or(0),active,t]).map_err(|e|if e.to_string().contains("UNIQUE"){ "Este SKU já está em uso.".into()}else{e.to_string()})?;Ok(conn.last_insert_rowid())}},
 "customers"=>save_person(&conn,"customers",input,t), "suppliers"=>save_person(&conn,"suppliers",input,t), _=>Err("Cadastro inválido".into()) }
}
fn save_person(conn:&Connection,table:&str,input:RecordInput,t:String)->Result<i64,String>{let sql=if table=="customers" {"INSERT INTO customers(name,phone,email,notes,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?5)"}else{"INSERT INTO suppliers(name,contact,phone,email,notes,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?6,?6)"};if let Some(id)=input.id{let update=if table=="customers"{"UPDATE customers SET name=?1,phone=?2,email=?3,notes=?4,updated_at=?5 WHERE id=?6"}else{"UPDATE suppliers SET name=?1,contact=?2,phone=?3,email=?4,notes=?5,updated_at=?6 WHERE id=?7"};let mut vals:Vec<&dyn rusqlite::ToSql>=vec![&input.name,&input.phone,&input.email,&input.notes,&t];if table=="suppliers"{vals.insert(1,&input.code)} vals.push(&id);conn.execute(update,vals.as_slice()).map_err(|e|e.to_string())?;Ok(id)}else{let mut vals:Vec<&dyn rusqlite::ToSql>=vec![&input.name,&input.phone,&input.email,&input.notes,&t];if table=="suppliers"{vals.insert(1,&input.code)}conn.execute(sql,vals.as_slice()).map_err(|e|e.to_string())?;Ok(conn.last_insert_rowid())}}

#[tauri::command]
fn delete_record(db: State<Database>, entity:String,id:i64)->Result<(),String>{let table=match entity.as_str(){"products"=>"products","customers"=>"customers","suppliers"=>"suppliers",_=>return Err("Cadastro inválido".into())};let conn=db.0.lock().map_err(|_|"Banco indisponível")?;conn.execute(&format!("DELETE FROM {table} WHERE id=?1"),params![id]).map_err(|e|if e.to_string().contains("FOREIGN KEY"){ "Este registro possui movimentações e não pode ser excluído.".into()}else{e.to_string()})?;Ok(())}

#[derive(Deserialize)] #[serde(rename_all="camelCase")] struct StockInput {product_id:i64,quantity:i64,kind:String,reason:String}
#[tauri::command] fn move_stock(db:State<Database>,input:StockInput)->Result<(),String>{if input.quantity<=0{return Err("Quantidade deve ser maior que zero.".into())}let mut conn=db.0.lock().map_err(|_|"Banco indisponível")?;let tx=conn.transaction().map_err(|e|e.to_string())?;let current:i64=tx.query_row("SELECT stock FROM products WHERE id=?1",params![input.product_id],|r|r.get(0)).map_err(|_|"Produto não encontrado.".to_string())?;let delta=if input.kind=="IN"{input.quantity}else{-input.quantity};if current+delta<0{return Err("Estoque insuficiente para esta saída.".into())}tx.execute("UPDATE products SET stock=stock+?1,updated_at=?2 WHERE id=?3",params![delta,now(),input.product_id]).map_err(|e|e.to_string())?;tx.execute("INSERT INTO stock_movements(product_id,type,quantity,reason,created_at)VALUES(?1,?2,?3,?4,?5)",params![input.product_id,input.kind,delta,input.reason,now()]).map_err(|e|e.to_string())?;tx.commit().map_err(|e|e.to_string())?;Ok(())}

#[derive(Deserialize)] #[serde(rename_all="camelCase")] struct SaleInput {customer_id:Option<i64>,payment_method:String,discount_cents:i64,items:Vec<SaleItemInput>}
#[derive(Deserialize)] #[serde(rename_all="camelCase")] struct SaleItemInput {product_id:i64,quantity:i64}
#[tauri::command] fn complete_sale(db:State<Database>,input:SaleInput)->Result<i64,String>{if input.items.is_empty(){return Err("Adicione ao menos um item.".into())}let mut conn=db.0.lock().map_err(|_|"Banco indisponível")?;let tx=conn.transaction().map_err(|e|e.to_string())?;let cash:Option<i64>=tx.query_row("SELECT id FROM cash_sessions WHERE status='OPEN'",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?;if cash.is_none(){return Err("Abra o caixa antes de concluir uma venda.".into())}let mut total=0;let mut priced=Vec::new();for item in &input.items{if item.quantity<=0{return Err("Quantidade inválida.".into())}let (stock,price):(i64,i64)=tx.query_row("SELECT stock,price_cents FROM products WHERE id=?1 AND active=1",params![item.product_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"Produto indisponível.".to_string())?;if stock<item.quantity{return Err("Estoque insuficiente em um dos itens.".into())}total+=price*item.quantity;priced.push((item,price));}let final_total=(total-input.discount_cents).max(0);tx.execute("INSERT INTO sales(customer_id,cash_session_id,total_cents,discount_cents,payment_method,created_at)VALUES(?1,?2,?3,?4,?5,?6)",params![input.customer_id,cash,final_total,input.discount_cents,input.payment_method,now()]).map_err(|e|e.to_string())?;let sale=tx.last_insert_rowid();for(item,price)in priced{tx.execute("INSERT INTO sale_items(sale_id,product_id,quantity,unit_price_cents,total_cents)VALUES(?1,?2,?3,?4,?5)",params![sale,item.product_id,item.quantity,price,price*item.quantity]).map_err(|e|e.to_string())?;tx.execute("UPDATE products SET stock=stock-?1,updated_at=?2 WHERE id=?3",params![item.quantity,now(),item.product_id]).map_err(|e|e.to_string())?;tx.execute("INSERT INTO stock_movements(product_id,type,quantity,reason,created_at)VALUES(?1,'SALE',?2,?3,?4)",params![item.product_id,-item.quantity,format!("Venda #{sale}"),now()]).map_err(|e|e.to_string())?;}tx.execute("INSERT INTO cash_movements(cash_session_id,type,amount_cents,description,created_at)VALUES(?1,'SALE',?2,?3,?4)",params![cash,final_total,format!("Venda #{sale}"),now()]).map_err(|e|e.to_string())?;tx.commit().map_err(|e|e.to_string())?;Ok(sale)}

#[tauri::command]
fn cash_action(db:State<Database>,action:String,amount_cents:i64,description:Option<String>)->Result<(),String>{
  let conn=db.0.lock().map_err(|_|"Banco indisponível")?;
  if amount_cents<0{return Err("Valor inválido.".into())}
  if action=="OPEN"{conn.execute("INSERT INTO cash_sessions(opened_at,opening_cents,status)VALUES(?1,?2,'OPEN')",params![now(),amount_cents]).map_err(|_|"Já existe um caixa aberto.".to_string())?;return Ok(())}
  let session:Option<i64>=conn.query_row("SELECT id FROM cash_sessions WHERE status='OPEN'",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
  let id=session.ok_or("Não há caixa aberto.")?;
  if action=="CLOSE"{
    let balance:i64=conn.query_row("SELECT cs.opening_cents+COALESCE(SUM(CASE WHEN cm.type='OUT' THEN -cm.amount_cents ELSE cm.amount_cents END),0) FROM cash_sessions cs LEFT JOIN cash_movements cm ON cm.cash_session_id=cs.id WHERE cs.id=?1",params![id],|r|r.get(0)).map_err(|e|e.to_string())?;
    conn.execute("UPDATE cash_sessions SET status='CLOSED',closed_at=?1,closing_cents=?2 WHERE id=?3",params![now(),balance,id]).map_err(|e|e.to_string())?;
  }else{
    let kind=if action=="IN"{"IN"}else{"OUT"};
    conn.execute("INSERT INTO cash_movements(cash_session_id,type,amount_cents,description,created_at)VALUES(?1,?2,?3,?4,?5)",params![id,kind,amount_cents,description.unwrap_or_else(||"Movimentação manual".into()),now()]).map_err(|e|e.to_string())?;
  }
  Ok(())
}

#[tauri::command] fn dashboard(db:State<Database>)->Result<Value,String>{let conn=db.0.lock().map_err(|_|"Banco indisponível")?;let today=Local::now().format("%Y-%m-%d").to_string();let sales:i64=conn.query_row("SELECT COUNT(*) FROM sales WHERE date(created_at)=?1",params![today],|r|r.get(0)).map_err(|e|e.to_string())?;let revenue:i64=conn.query_row("SELECT COALESCE(SUM(total_cents),0) FROM sales WHERE date(created_at)=?1",params![today],|r|r.get(0)).map_err(|e|e.to_string())?;let low:i64=conn.query_row("SELECT COUNT(*) FROM products WHERE stock>0 AND stock<=minimum_stock",[],|r|r.get(0)).map_err(|e|e.to_string())?;let out:i64=conn.query_row("SELECT COUNT(*) FROM products WHERE stock=0",[],|r|r.get(0)).map_err(|e|e.to_string())?;let cash:i64=conn.query_row("SELECT COALESCE(opening_cents,0)+COALESCE((SELECT SUM(CASE WHEN type='OUT' THEN -amount_cents ELSE amount_cents END) FROM cash_movements WHERE cash_session_id=cash_sessions.id),0) FROM cash_sessions WHERE status='OPEN'",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?.unwrap_or(0);Ok(json!({"revenue":revenue,"sales":sales,"averageTicket":if sales>0{revenue/sales}else{0},"lowStock":low,"outOfStock":out,"cash":cash,"recentSales":rows(&conn,"SELECT s.id,s.total_cents,s.created_at,COALESCE(c.name,'Consumidor final') customer_name FROM sales s LEFT JOIN customers c ON c.id=s.customer_id ORDER BY s.id DESC LIMIT 5",&[])?}))}

#[tauri::command] fn global_search(db:State<Database>,query:String)->Result<Vec<Value>,String>{let conn=db.0.lock().map_err(|_|"Banco indisponível")?;let t=format!("%{query}%");rows(&conn,"SELECT 'Produto' type,id,name subtitle FROM products WHERE name LIKE ?1 OR sku LIKE ?1 UNION ALL SELECT 'Cliente',id,name,COALESCE(phone,'') FROM customers WHERE name LIKE ?1 UNION ALL SELECT 'Fornecedor',id,name,COALESCE(phone,'') FROM suppliers WHERE name LIKE ?1 LIMIT 20",&[&t])}

#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct AccountInput { id:Option<i64>, kind:String, description:String, person_name:Option<String>, category:Option<String>, due_date:String, amount_cents:i64 }

#[tauri::command]
fn save_account(db:State<Database>,input:AccountInput)->Result<i64,String>{
  if input.description.trim().is_empty()||input.due_date.trim().is_empty(){return Err("Informe descrição e vencimento.".into())}
  if input.amount_cents<=0{return Err("Informe um valor maior que zero.".into())}
  if input.kind!="PAYABLE"&&input.kind!="RECEIVABLE"{return Err("Tipo de conta inválido.".into())}
  let conn=db.0.lock().map_err(|_|"Banco indisponível")?;
  if let Some(id)=input.id{
    let status:String=conn.query_row("SELECT status FROM accounts WHERE id=?1",params![id],|r|r.get(0)).map_err(|_|"Conta não encontrada.".to_string())?;
    if status!="PENDING" {return Err("Somente contas pendentes podem ser editadas.".into())}
    conn.execute("UPDATE accounts SET kind=?1,description=?2,person_name=?3,category=?4,due_date=?5,amount_cents=?6 WHERE id=?7",params![input.kind,input.description,input.person_name,input.category,input.due_date,input.amount_cents,id]).map_err(|e|e.to_string())?;
    Ok(id)
  }else{
    conn.execute("INSERT INTO accounts(kind,description,person_name,category,due_date,amount_cents,status,created_at)VALUES(?1,?2,?3,?4,?5,?6,'PENDING',?7)",params![input.kind,input.description,input.person_name,input.category,input.due_date,input.amount_cents,now()]).map_err(|e|e.to_string())?;
    Ok(conn.last_insert_rowid())
  }
}

#[tauri::command]
fn settle_account(db:State<Database>,id:i64,action:String)->Result<(),String>{
  let mut conn=db.0.lock().map_err(|_|"Banco indisponível")?;
  let tx=conn.transaction().map_err(|e|e.to_string())?;
  let (kind,description,amount,status):(String,String,i64,String)=tx.query_row("SELECT kind,description,amount_cents,status FROM accounts WHERE id=?1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"Conta não encontrada.".to_string())?;
  if status!="PENDING"{return Err("Esta conta já foi processada.".into())}
  if action=="CANCEL"{tx.execute("UPDATE accounts SET status='CANCELED' WHERE id=?1",params![id]).map_err(|e|e.to_string())?;tx.commit().map_err(|e|e.to_string())?;return Ok(())}
  let expected=if kind=="PAYABLE"{"PAY"}else{"RECEIVE"};if action!=expected{return Err("Ação incompatível com o tipo da conta.".into())}
  let new_status=if kind=="PAYABLE"{"PAID"}else{"RECEIVED"};
  tx.execute("UPDATE accounts SET status=?1,paid_at=?2 WHERE id=?3",params![new_status,now(),id]).map_err(|e|e.to_string())?;
  let cash:Option<i64>=tx.query_row("SELECT id FROM cash_sessions WHERE status='OPEN'",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
  if let Some(cash_id)=cash {let cash_type=if kind=="PAYABLE"{"OUT"}else{"IN"};tx.execute("INSERT INTO cash_movements(cash_session_id,type,amount_cents,description,created_at)VALUES(?1,?2,?3,?4,?5)",params![cash_id,cash_type,amount,format!("Financeiro: {description}"),now()]).map_err(|e|e.to_string())?;}
  tx.commit().map_err(|e|e.to_string())?;Ok(())
}

#[tauri::command]
fn delete_account(db:State<Database>,id:i64)->Result<(),String>{let conn=db.0.lock().map_err(|_|"Banco indisponível")?;let status:String=conn.query_row("SELECT status FROM accounts WHERE id=?1",params![id],|r|r.get(0)).map_err(|_|"Conta não encontrada.".to_string())?;if status!="PENDING"{return Err("Somente contas pendentes podem ser excluídas.".into())}conn.execute("DELETE FROM accounts WHERE id=?1",params![id]).map_err(|e|e.to_string())?;Ok(())}

#[tauri::command]
fn finance_summary(db:State<Database>)->Result<Value,String>{let conn=db.0.lock().map_err(|_|"Banco indisponível")?;let today=Local::now().format("%Y-%m-%d").to_string();let value=|sql:&str|conn.query_row(sql,params![today],|r|r.get::<_,i64>(0)).map_err(|e|e.to_string());Ok(json!({"payable":value("SELECT COALESCE(SUM(amount_cents),0) FROM accounts WHERE kind='PAYABLE' AND status='PENDING'")?,"receivable":value("SELECT COALESCE(SUM(amount_cents),0) FROM accounts WHERE kind='RECEIVABLE' AND status='PENDING'")?,"overdue":value("SELECT COALESCE(SUM(amount_cents),0) FROM accounts WHERE status='PENDING' AND due_date < ?1")?,"paid":value("SELECT COALESCE(SUM(amount_cents),0) FROM accounts WHERE kind='PAYABLE' AND status='PAID'")?,"received":value("SELECT COALESCE(SUM(amount_cents),0) FROM accounts WHERE kind='RECEIVABLE' AND status='RECEIVED'")?}))}

#[tauri::command]
fn get_company(db:State<Database>)->Result<Value,String>{let conn=db.0.lock().map_err(|_|"Banco indisponível")?;rows(&conn,"SELECT name,COALESCE(cnpj,'') cnpj,COALESCE(phone,'') phone,COALESCE(address,'') address,COALESCE(logo_path,'') logo_path FROM company WHERE id=1",&[])?.into_iter().next().ok_or("Empresa não encontrada".into())}

#[derive(Deserialize)] #[serde(rename_all="camelCase")]
struct CompanyInput{name:String,cnpj:Option<String>,phone:Option<String>,address:Option<String>,logo_path:Option<String>}
#[tauri::command]
fn save_company(db:State<Database>,input:CompanyInput)->Result<(),String>{if input.name.trim().is_empty(){return Err("Informe o nome da empresa.".into())}let conn=db.0.lock().map_err(|_|"Banco indisponível")?;conn.execute("UPDATE company SET name=?1,cnpj=?2,phone=?3,address=?4,logo_path=?5,updated_at=?6 WHERE id=1",params![input.name,input.cnpj,input.phone,input.address,input.logo_path,now()]).map_err(|e|e.to_string())?;Ok(())}
#[derive(Deserialize)] #[serde(rename_all="camelCase")] struct PreferenceInput { currency:String, date_format:String, appearance:String }
#[tauri::command]
fn get_preferences(db:State<Database>)->Result<Value,String>{let conn=db.0.lock().map_err(|_|"Banco indisponível")?;let entries=rows(&conn,"SELECT key,value FROM settings",&[])?;let mut result=serde_json::Map::new();for entry in entries{if let Value::Object(map)=entry{if let(Some(Value::String(k)),Some(Value::String(v)))=(map.get("key"),map.get("value")){result.insert(k.clone(),Value::String(v.clone()));}}}Ok(Value::Object(result))}
#[tauri::command]
fn save_preferences(db:State<Database>,input:PreferenceInput)->Result<(),String>{if input.currency!="BRL"||input.date_format!="DD/MM/AAAA"||(input.appearance!="dark"&&input.appearance!="system"){return Err("Preferência inválida.".into())}let conn=db.0.lock().map_err(|_|"Banco indisponível")?;let t=now();for(k,v)in [("currency",input.currency),("date_format",input.date_format),("appearance",input.appearance)]{conn.execute("INSERT INTO settings(key,value,updated_at)VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![k,v,t]).map_err(|e|e.to_string())?;}Ok(())}

fn verify_backup(path:&str)->Result<Connection,String>{let conn=Connection::open(path).map_err(|_|"Não foi possível abrir o arquivo de backup.".to_string())?;let integrity:String=conn.query_row("PRAGMA integrity_check",[],|r|r.get(0)).map_err(|_|"Backup inválido.".to_string())?;if integrity!="ok"{return Err("O arquivo informado não possui integridade SQLite.".into())}let has:i64=conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='users'",[],|r|r.get(0)).map_err(|_|"Backup inválido.".to_string())?;if has==0{return Err("O arquivo não é um backup do Gestor Comercial.".into())}Ok(conn)}
#[tauri::command]
fn create_backup(db:State<Database>,path:String)->Result<(),String>{if Path::new(&path).exists(){return Err("Já existe um arquivo neste local. Escolha outro nome.".into())}let source=db.0.lock().map_err(|_|"Banco indisponível")?;let mut target=Connection::open(&path).map_err(|_|"Não foi possível criar o arquivo de backup.".to_string())?;let backup=Backup::new(&source,&mut target).map_err(|e|e.to_string())?;backup.run_to_completion(100,Duration::from_millis(5),None).map_err(|_|"Falha ao criar backup.".to_string())?;verify_backup(&path)?;Ok(())}
#[tauri::command]
fn restore_backup(db:State<Database>,path:String)->Result<(),String>{let source=verify_backup(&path)?;let mut target=db.0.lock().map_err(|_|"Banco indisponível")?;let backup=Backup::new(&source,&mut target).map_err(|e|e.to_string())?;backup.run_to_completion(100,Duration::from_millis(5),None).map_err(|_|"Falha ao restaurar backup.".to_string())?;drop(backup);migrate(&target).map_err(|e|e.to_string())?;Ok(())}

pub fn run(){tauri::Builder::default().plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_fs::init()).setup(|app|{let path=app.path().app_data_dir().map_err(|e|e.to_string())?;std::fs::create_dir_all(&path).map_err(|e|e.to_string())?;let conn=Connection::open(path.join("gestor.db")).map_err(|e|e.to_string())?;migrate(&conn).map_err(|e|e.to_string())?;app.manage(Database(Mutex::new(conn)));Ok(())}).invoke_handler(tauri::generate_handler![login,list_records,save_record,delete_record,move_stock,complete_sale,cash_action,dashboard,global_search,save_account,settle_account,delete_account,finance_summary,get_company,save_company,get_preferences,save_preferences,create_backup,restore_backup]).run(tauri::generate_context!()).expect("erro ao executar o aplicativo")}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn migration_creates_persistent_financial_structure() {
    let conn=Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute("INSERT INTO accounts(kind,description,due_date,amount_cents,status,created_at) VALUES('PAYABLE','Compra de mercadorias','2026-09-10',1590,'PENDING',?1)",params![now()]).unwrap();
    let amount:i64=conn.query_row("SELECT amount_cents FROM accounts",[],|r|r.get(0)).unwrap();
    let currency:String=conn.query_row("SELECT value FROM settings WHERE key='currency'",[],|r|r.get(0)).unwrap();
    assert_eq!(amount,1590);
    assert_eq!(currency,"BRL");
  }

  #[test]
  fn sqlite_backup_restores_original_data() {
    let source=Connection::open_in_memory().unwrap();
    migrate(&source).unwrap();
    source.execute("UPDATE company SET name='Comércio Original' WHERE id=1",[]).unwrap();
    let path=std::env::temp_dir().join(format!("gestor-backup-{}.db",std::process::id()));
    let _=std::fs::remove_file(&path);
    let mut file=Connection::open(&path).unwrap();
    let backup=Backup::new(&source,&mut file).unwrap();
    backup.run_to_completion(100,Duration::from_millis(1),None).unwrap();
    drop(backup);
    source.execute("UPDATE company SET name='Comércio Alterado' WHERE id=1",[]).unwrap();
    let original=Connection::open(&path).unwrap();
    let mut target=source;
    let restore=Backup::new(&original,&mut target).unwrap();
    restore.run_to_completion(100,Duration::from_millis(1),None).unwrap();
    drop(restore);
    let name:String=target.query_row("SELECT name FROM company WHERE id=1",[],|r|r.get(0)).unwrap();
    assert_eq!(name,"Comércio Original");
    let _=std::fs::remove_file(path);
  }
}
