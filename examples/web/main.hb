use './syntax' -> *
use './lib'
    -> { hanabi }
    -> { Response }

fn main(){
    let app = hanabi(
        route {
            GET '/' {
                Response::text('Hello!')
            }
            GET '/status' {
                Response::json({
                    status: 'OK'
                    code: 200
                })
            }
            GET '/{id}/status' {
                Response::json({
                    status: 'OK'
                    code: 200
                    message: `Hello! {id.get()}`
                })
            }
            @durable_object('ACCOUNT') -> account
            GET `/{id}/status?time={time: 'day' | 'night'}` {
                let account = account.get(id)
                account.stamp(Clock::now())
                Response::json({
                    status: 'OK'
                    code: 200
                    message: `{if time.get() == 'day' {'Hello'} else {'Good evening'}}! {id.get()}`
                })
            }
        }
    )

}
