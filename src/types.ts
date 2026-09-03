export type User = { id:number; name:string; username:string };
export type Product = { id:number; name:string; sku:string; category?:string; supplier_id?:number; supplier_name?:string; cost_cents:number; price_cents:number; stock:number; minimum_stock:number; active:number };
export type Customer = { id:number; name:string; phone?:string; email?:string; notes?:string; total_purchases:number; last_purchase?:string };
export type Supplier = { id:number; name:string; contact?:string; phone?:string; email?:string; notes?:string };
export type Dashboard = { revenue:number; sales:number; averageTicket:number; cash:number; lowStock:number; outOfStock:number; recentSales:{id:number;total_cents:number;created_at:string;customer_name:string}[] };
export type CartItem = Product & { quantity:number };
