//
//  Alert.swift
//  waterui-swift
//
//  Created by Lexo Liu on 11/6/24.
//


import AppKit
func alert(){
    let alert = NS(title: "My Alert", message: "This is an alert.", preferredStyle: .alert)
    alert.addAction(UIAlertAction(title: NSLocalizedString("OK", comment: "Default action"), style: .default, handler: { _ in
    NSLog("The \"OK\" alert occured.")
    }))
    self.present(alert, animated: true, completion: nil)
}
