# Changes

## [0.2.0] - 2021-01-xx

* Use ntex-codec 0.2

* Use ahash instead of fxhash

## [codec-0.3.0] - 2021-01-xx

* Use ntex-codec 0.2


## [0.1.22] - 2020-12-19

* Support partial transfers on receiver side

## [0.1.21] - 2020-12-14

* Split large message into smaller transfers

## [0.1.20] - 2020-11-25

* Do not log error for remote closed connections

## [0.1.19] - 2020-10-23

* Fix flow frame handling

* Use proper handle for sender link

## [codec-0.2.1] - 2020-09-17

* Do not add empty Message section to encoded buffer

## [codec-0.2.0] - 2020-08-05

* Drop In/OutMessage

* Use vec for message annotations and message app propperties

## [0.1.17] - 2020-08-04

* Rename server::Message to server::Transfer

## [codec-0.1.4] - 2020-08-04

* Deprecated In/OutMessage, replaced with Message

## [0.1.16] - 2020-07-31

* Add receiver/receiver_mut for server Link

## [0.1.15] - 2020-07-25

* Fix sender link apply flow

## [0.1.14] - 2020-07-25

* Notify sender link detached

## [0.1.13] - 2020-07-23

* Better logging

## [0.1.10] - 2020-05-12

* Add AttachReceiver control frame

## [0.1.9] - 2020-05-11

* Add standard error code constants

## [0.1.8] - 2020-05-04

* Proper handling of errors during sender link opening

## [0.1.7] - 2020-05-02

* Add `LinkError::redirect()`

## [codec-0.1.2] - 2020-05-02

* Add const `Symbol::from_static()` helper method.

## [0.1.5] - 2020-04-28

* Fix open multiple sessions

## [0.1.4] - 2020-04-21

* Refactor server control frame

* Wakeup receiver link on disconnect

## [0.1.3] - 2020-04-21

* Fix OutMessage and InMessage encoding

* Move LinkError to root

## [0.1.2] - 2020-04-20

* Fix handshake timeout

* Propagate receiver remote close errors

## [0.1.1] - 2020-04-14

* Handle detach during reciver link open

## [0.1.0] - 2020-04-01

* Switch to ntex

## [0.1.4] - 2020-03-05

* Add server handshake timeout

## [0.1.3] - 2020-02-10

* Allow to override sender link attach frame

## [0.1.2] - 2019-12-25

* Allow to specify multi-pattern for topics

## [0.1.1] - 2019-12-18

* Separate control frame entries for detach sender qand detach receiver

* Proper detach remote receiver

* Replace `async fn` with `impl Future`

## [0.1.0] - 2019-12-11

* Initial release
