import React, {useState, useEffect} from 'react';
import logo from './logo.svg';
import './App.css';
import Version from "./Version";
import {CssBaseline} from "@material-ui/core";

const App: React.FC = () => {

    return (
        <div>
            <CssBaseline/>
            <Version/>
        </div>
    );
}

export default App;
