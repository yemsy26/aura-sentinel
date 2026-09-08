use spectrasat_core::solve_native_rust;
fn main() {
    let unsat: Vec<Vec<i32>> = vec![
        vec![1,2,2],vec![-1,-2,-2],vec![2,3,3],vec![-2,-3,-3],
        vec![3,4,4],vec![-3,-4,-4],vec![4,5,5],vec![-4,-5,-5],
        vec![5,1,1],vec![-5,-1,-1]
    ];
    println!("Test 1 (UNSAT): {}", solve_native_rust(5, unsat));
    let sat: Vec<Vec<i32>> = vec![
        vec![1,2,3],vec![-1,-2,4],vec![3,-4,5],vec![-3,4,-5],
        vec![2,-5,6],vec![-2,5,-6],vec![6,7,8],vec![-6,-7,-8],
        vec![1,-3,7],vec![-1,3,-7],vec![4,6,-8],vec![-4,-6,8],
        vec![2,4,7],vec![-2,-4,-7],vec![1,5,8],vec![-1,-5,-8],
        vec![3,6,-7],vec![-3,-6,7]
    ];
    println!("Test 2 (SAT):   {}", solve_native_rust(8, sat));
}
