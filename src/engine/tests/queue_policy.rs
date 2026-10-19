use super::*;
fn records()->Vec<Record> {
    (1..=5).map(|n|crate::engine::tests::review_record(&format!("{n:040x}"),Path::new("/fixture"))).collect()
}
#[test]
fn admission_with_force() {
    let mut rows=records();rows[1].force=true;
    let stats=vec![Some(Stats{paused:true,..Default::default()});5];
    assert_eq!(admit(&rows,&stats,1),[true,true,false,false,false]);
    rows[0].paused=true;
    assert_eq!(admit(&rows,&stats,1),[false,true,true,false,false]);
    rows[1].force=false;
    assert_eq!(admit(&rows,&stats,1),[false,true,false,false,false]);
    rows[1].staged=true;
    assert_eq!(admit(&rows,&stats,1),[false,false,true,false,false]);
}
#[test]
fn reorder_positions() {
    let mut rows=records();let running=rows[0].hash.clone();let third=rows[2].hash.clone();
    rows[1].force=true;rows[3].paused=true;rows[4].completion_on=123;
    let mut ledger=Ledger{records:rows,..Default::default()};
    assert_eq!(queue_policy::move_record(&mut ledger,&third,Direction::Top).unwrap(),1);
    assert_eq!(queue_policy::position(&ledger.records,&running),Some(2));
    let stats:Vec<_>=ledger.records.iter().map(|r|Some(Stats{paused:r.hash!=running,pinned:r.hash==running,..Default::default()})).collect();
    let run=admit(&ledger.records,&stats,1);
    assert!(!run[0]);assert!(run[1]);assert!(run[2]);
    assert_eq!(queue_policy::move_record(&mut ledger,&third,Direction::Bottom).unwrap(),2);
}
#[test]
fn up_skips_unpositioned() {
    let mut rows=records();rows[1].paused=true;rows[2].force=true;rows[3].completion_on=123;
    let target=rows[4].hash.clone();let paused=rows[1].hash.clone();let forced=rows[2].hash.clone();
    let mut ledger=Ledger{records:rows,..Default::default()};
    assert_eq!(queue_policy::move_record(&mut ledger,&target,Direction::Up).unwrap(),1);
    assert_eq!(ledger.records[1].hash,paused);assert_eq!(ledger.records[2].hash,forced);
    assert!(queue_policy::move_record(&mut ledger,&forced,Direction::Top).is_err());
    ledger.records[0].completion_on=-1;
    assert_eq!(queue_policy::position(&ledger.records,&target),Some(1));
}
