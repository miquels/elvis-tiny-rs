//! `move3.c`: character searches (`f`, `F`, `t`, `T`, `;`, `,`).

/// Stands in for the C function pointers `prevfwdfn`/`prevrevfn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSearch {
    Fch,
    #[allow(non_camel_case_types)]
    F_ch,
    Tch,
    #[allow(non_camel_case_types)]
    T_ch,
}

#[derive(Default)]
pub struct Move3 {
    pub prevfwdfn: Option<CharSearch>,
    pub prevrevfn: Option<CharSearch>,
    pub prev_key: i32,
}
