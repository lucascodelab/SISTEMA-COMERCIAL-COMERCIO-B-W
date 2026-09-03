import { invoke } from "@tauri-apps/api/core";
export const api = {
  login: (username:string,password:string) => invoke("login",{username,password}),
  list: <T>(entity:string,search?:string) => invoke<T[]>("list_records",{entity,search}),
  save: (entity:string,input:object) => invoke<number>("save_record",{entity,input}),
  remove: (entity:string,id:number) => invoke("delete_record",{entity,id}),
  moveStock:(input:object)=>invoke("move_stock",{input}), sale:(input:object)=>invoke<number>("complete_sale",{input}),
  cash:(action:string,amountCents:number,description?:string)=>invoke("cash_action",{action,amountCents,description}),
  dashboard:<T>()=>invoke<T>("dashboard"), search:<T>(query:string)=>invoke<T[]>("global_search",{query}),
  saveAccount:(input:object)=>invoke<number>("save_account",{input}), settleAccount:(id:number,action:string)=>invoke("settle_account",{id,action}), deleteAccount:(id:number)=>invoke("delete_account",{id}), financeSummary:<T>()=>invoke<T>("finance_summary"),
  company:<T>()=>invoke<T>("get_company"), saveCompany:(input:object)=>invoke("save_company",{input}),
  backup:(path:string)=>invoke("create_backup",{path}), restore:(path:string)=>invoke("restore_backup",{path})
  ,preferences:<T>()=>invoke<T>("get_preferences"), savePreferences:(input:object)=>invoke("save_preferences",{input})
};
